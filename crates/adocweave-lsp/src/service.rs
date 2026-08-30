//! Runtime-independent language features over owned document analyses.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adocweave::CancellationCheck;
use adocweave::SourceId;
use adocweave::output::diagnostics::{RuleSettings, lint_rule};
use adocweave::text::SourceDocument;
use adocweave_project::{
    ConfigSelection, ProjectAuthority, ProjectError, ProjectExpansionError, ProjectLimits,
    ProjectObservationAccess, ProjectOverrides, ProjectRequest, ProjectResourceErrorCode,
    ProjectResourceKind, ProjectResourceOutcome, ProjectResourceResult, ProjectResourceSelection,
    ProjectResult, ProjectSource, ProjectTarget, ProjectTargetError,
};
use async_lsp::lsp_types as lsp;
use serde::Deserialize;

use crate::cancellation::{QueryCancellation, QueryResult};
use crate::diagnostics::QuickFixCapabilities;
use crate::document_symbols::SymbolPresentation;
use crate::editing;
use crate::hover::HoverPresentation;
use crate::navigation::{self, NavigationInput};
use crate::position::{PositionEncoding, lsp_position_to_core, negotiate_encoding, request_offset};
use crate::presentation;
use crate::state::DocumentStore;
use crate::state::{
    AdoptedAnalysis, Adoption, DocumentSnapshot, ExpandedDocumentAnalysis, PreparedProjectRequest,
    ProjectAnalysisSnapshot, ProjectProblem, ProjectSourceIndex, ProjectSourceState,
};
use crate::{SERVER_NAME, VERSION};

pub(crate) const MAX_WORKSPACE_WATCH_CHANGES: usize = 10_000;
const MAX_WORKSPACE_WATCH_URI_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClientProfile {
    hover: HoverPresentation,
    hierarchical_document_symbols: bool,
    code_action_quickfix: bool,
    code_action_is_preferred: bool,
    versioned_document_changes: bool,
    diagnostic_version: bool,
    document_link_tooltip: bool,
    semantic_tokens_full: bool,
    rename_prepare_support: bool,
    workspace_folders: bool,
    watched_files_dynamic_registration: bool,
}

impl Default for ClientProfile {
    fn default() -> Self {
        Self {
            hover: HoverPresentation::Markdown,
            hierarchical_document_symbols: true,
            code_action_quickfix: true,
            code_action_is_preferred: true,
            versioned_document_changes: true,
            diagnostic_version: true,
            document_link_tooltip: true,
            semantic_tokens_full: true,
            rename_prepare_support: true,
            workspace_folders: false,
            watched_files_dynamic_registration: false,
        }
    }
}

impl ClientProfile {
    fn from_capabilities(capabilities: &lsp::ClientCapabilities) -> Self {
        let text_document = capabilities.text_document.as_ref();
        let workspace = capabilities.workspace.as_ref();
        let hover = text_document
            .and_then(|capabilities| capabilities.hover.as_ref())
            .and_then(|capabilities| capabilities.content_format.as_ref())
            .and_then(|formats| {
                formats.iter().find_map(|format| {
                    if format == &lsp::MarkupKind::Markdown {
                        Some(HoverPresentation::Markdown)
                    } else if format == &lsp::MarkupKind::PlainText {
                        Some(HoverPresentation::PlainText)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();
        let code_action = text_document.and_then(|capabilities| capabilities.code_action.as_ref());
        let code_action_quickfix = code_action
            .and_then(|capabilities| capabilities.code_action_literal_support.as_ref())
            .is_some_and(|support| {
                support
                    .code_action_kind
                    .value_set
                    .iter()
                    .any(|kind| kind == lsp::CodeActionKind::QUICKFIX.as_str())
            });
        Self {
            hover,
            hierarchical_document_symbols: text_document
                .and_then(|capabilities| capabilities.document_symbol.as_ref())
                .and_then(|capabilities| capabilities.hierarchical_document_symbol_support)
                == Some(true),
            code_action_quickfix,
            code_action_is_preferred: code_action
                .and_then(|capabilities| capabilities.is_preferred_support)
                == Some(true),
            versioned_document_changes: workspace
                .and_then(|capabilities| capabilities.workspace_edit.as_ref())
                .and_then(|capabilities| capabilities.document_changes)
                == Some(true),
            diagnostic_version: text_document
                .and_then(|capabilities| capabilities.publish_diagnostics.as_ref())
                .and_then(|capabilities| capabilities.version_support)
                == Some(true),
            document_link_tooltip: text_document
                .and_then(|capabilities| capabilities.document_link.as_ref())
                .and_then(|capabilities| capabilities.tooltip_support)
                == Some(true),
            semantic_tokens_full: text_document
                .and_then(|capabilities| capabilities.semantic_tokens.as_ref())
                .is_some_and(|capabilities| {
                    capabilities.requests.full.as_ref().is_some_and(|full| {
                        matches!(
                            full,
                            lsp::SemanticTokensFullOptions::Bool(true)
                                | lsp::SemanticTokensFullOptions::Delta { .. }
                        )
                    }) && capabilities.formats.contains(&lsp::TokenFormat::RELATIVE)
                        && capabilities
                            .token_types
                            .contains(&lsp::SemanticTokenType::STRING)
                        && capabilities
                            .token_types
                            .contains(&lsp::SemanticTokenType::VARIABLE)
                }),
            rename_prepare_support: text_document
                .and_then(|capabilities| capabilities.rename.as_ref())
                .and_then(|capabilities| capabilities.prepare_support)
                == Some(true),
            workspace_folders: workspace.and_then(|capabilities| capabilities.workspace_folders)
                == Some(true),
            watched_files_dynamic_registration: workspace
                .and_then(|capabilities| capabilities.did_change_watched_files)
                .and_then(|capabilities| capabilities.dynamic_registration)
                == Some(true),
        }
    }
}

/// Long-lived semantic state for one Language Server connection.
///
/// Runtime resources such as task handles stay in the protocol backend. The
/// current documents, configuration, project roots, cancellation tokens and
/// adopted results have exactly this owner.
#[derive(Clone)]
pub(crate) struct Session {
    #[cfg(not(test))]
    documents: DocumentStore,
    #[cfg(test)]
    pub(crate) documents: DocumentStore,
    pub position_encoding: PositionEncoding,
    client: ClientProfile,
    settings: ServerSettings,
    workspace_roots: std::collections::BTreeMap<String, lsp::Url>,
    project_observations: BTreeMap<String, ProjectObservations>,
    pending_project_observations: BTreeMap<String, PendingProjectObservations>,
}

#[derive(Clone)]
struct ProjectObservations {
    uris: BTreeSet<String>,
}

#[derive(Clone)]
struct PendingProjectObservations {
    generation: u64,
    uris: BTreeSet<String>,
}

pub(crate) enum ProjectAnalysisOutcome {
    Processed(Result<ProjectResult, ProjectError>),
    Rejected(ProjectProblem),
}

pub(crate) struct ProjectAnalysisCompletion {
    pub(crate) snapshot: ProjectAnalysisSnapshot,
    pub(crate) outcome: ProjectAnalysisOutcome,
    pub(crate) source_index: ProjectSourceIndex,
    pub(crate) observation_access: Option<ProjectObservationAccess>,
    pub(crate) observations_are_current: Option<bool>,
}

pub(crate) enum ProjectAnalysisAction {
    Validate(Box<ProjectAnalysisCompletion>),
    Retry(ProjectAnalysisSnapshot),
    Publish {
        snapshot: ProjectAnalysisSnapshot,
        diagnostic_uris: Vec<String>,
    },
    Ignore,
}

pub(crate) struct SessionCloseOutcome {
    pub closed: bool,
    pub reanalysis_jobs: Vec<ProjectAnalysisSnapshot>,
    pub diagnostic_uris: std::collections::BTreeSet<String>,
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("documents", &self.documents)
            .field("position_encoding", &self.position_encoding)
            .field("client", &self.client)
            .field("settings", &self.settings)
            .field("workspace_roots", &self.workspace_roots)
            .finish()
    }
}

impl Default for Session {
    fn default() -> Self {
        Self {
            documents: DocumentStore::default(),
            position_encoding: PositionEncoding::Utf16,
            client: ClientProfile::default(),
            settings: ServerSettings::default(),
            workspace_roots: std::collections::BTreeMap::new(),
            project_observations: BTreeMap::new(),
            pending_project_observations: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct ServerSettings {
    debounce_ms: u64,
    enabled_rules: Vec<String>,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            debounce_ms: 30,
            enabled_rules: Vec::new(),
        }
    }
}

fn reject_unsupported_uri(job: &mut ProjectAnalysisSnapshot, uri: &lsp::Url) -> bool {
    if uri.scheme() == "file" {
        return false;
    }
    job.project_problem = Some(ProjectProblem {
        document_uri: Some(job.uri.clone()),
        range: zero_text_range(),
        code: "unsupported-uri".to_owned(),
        message: format!(
            "The URI scheme '{}' is not supported. Only file URIs can be analyzed.",
            uri.scheme()
        ),
    });
    true
}

fn zero_text_range() -> adocweave::text::TextRange {
    adocweave::text::TextRange::new(
        adocweave::text::TextSize::ZERO,
        adocweave::text::TextSize::ZERO,
    )
    .expect("zero range")
}

fn target_problem(job: &ProjectAnalysisSnapshot, error: ProjectTargetError) -> ProjectProblem {
    let code = match &error {
        ProjectTargetError::Read(_) => "project-read-error",
        ProjectTargetError::Parse(_) => "project-parse-error",
        ProjectTargetError::EditConflict(_) => "project-edit-conflict",
        ProjectTargetError::Incomplete(_) => "project-limit",
    };
    ProjectProblem {
        document_uri: Some(job.uri.clone()),
        range: zero_text_range(),
        code: code.to_owned(),
        message: error.to_string(),
    }
}

fn project_error_problem(uri: &str, error: &ProjectError) -> ProjectProblem {
    ProjectProblem {
        document_uri: Some(uri.to_owned()),
        range: zero_text_range(),
        code: match error {
            ProjectError::Cancelled => "cancelled",
            ProjectError::Config(_) => "project-config-error",
            ProjectError::TargetSelection(_) => "project-target-error",
            ProjectError::Authority(_) => "project-authority-error",
            ProjectError::InvalidInput(_) => "project-input-error",
            ProjectError::Limit(_) => "project-limit",
        }
        .to_owned(),
        message: error.to_string(),
    }
}

fn expansion_problem(
    job: &ProjectAnalysisSnapshot,
    sources: &ProjectSourceIndex,
    resources: &[ProjectResourceResult],
    error: ProjectExpansionError,
) -> ProjectProblem {
    if let ProjectExpansionError::Preprocess(error) = &error {
        return ProjectProblem {
            document_uri: error
                .source_id
                .as_ref()
                .and_then(|source_id| sources.get(source_id))
                .map(|source| source.uri.clone())
                .or_else(|| Some(job.uri.clone())),
            range: error.range,
            code: error.kind.as_str().to_owned(),
            message: error.message.clone(),
        };
    }
    let code = match &error {
        ProjectExpansionError::Resource(error) => match error.code {
            ProjectResourceErrorCode::Missing => "missing-resource",
            ProjectResourceErrorCode::OutsideAuthority => "unsafe-target",
            ProjectResourceErrorCode::Limit => "project-limit",
            ProjectResourceErrorCode::PermissionDenied
            | ProjectResourceErrorCode::InvalidUtf8
            | ProjectResourceErrorCode::InvalidPath
            | ProjectResourceErrorCode::ReadFailed
            | ProjectResourceErrorCode::Unverifiable => "project-resource-error",
        },
        ProjectExpansionError::Options(_) => "project-options-error",
        ProjectExpansionError::Preprocess(_) => unreachable!(),
        ProjectExpansionError::Parse(_) => "project-parse-error",
        ProjectExpansionError::Projection(_) => "project-source-mapping-error",
        ProjectExpansionError::Incomplete(_) => "project-limit",
    };
    let resource_request = match &error {
        ProjectExpansionError::Resource(error) => resources.iter().rev().find(|resource| {
            resource.kind == ProjectResourceKind::Include
                && resource
                    .requested_at
                    .as_ref()
                    .is_some_and(|location| location.range.is_some())
                && error
                    .path
                    .as_ref()
                    .is_none_or(|path| path == &resource.path || path == &resource.requested_path)
        }),
        _ => None,
    };
    ProjectProblem {
        document_uri: resource_request
            .and_then(|resource| resource.requested_at.as_ref())
            .and_then(|location| sources.get(&location.source_id))
            .map(|source| source.uri.clone())
            .or_else(|| Some(job.uri.clone())),
        range: resource_request
            .and_then(|resource| resource.requested_at.as_ref()?.range)
            .unwrap_or_else(zero_text_range),
        code: code.to_owned(),
        message: error.to_string(),
    }
}

pub(crate) fn project_observations_are_current(
    result: &Result<ProjectResult, ProjectError>,
    access: &ProjectObservationAccess,
    cancellation: &dyn CancellationCheck,
) -> bool {
    let Ok(mut session) = access.session() else {
        return false;
    };
    let mut observe = |candidate: &adocweave_project::ProjectObservationCandidate| {
        if cancellation.is_cancelled()
            || session.observe(&candidate.path, candidate.kind) != candidate.observation
        {
            return false;
        }
        true
    };
    match result {
        Ok(result) => result
            .resources
            .iter()
            .chain(result.targets.iter().flat_map(|target| &target.resources))
            .filter_map(|resource| resource.observation.as_ref())
            .all(&mut observe),
        Err(error) => error.repair_candidate().is_none_or(observe),
    }
}

fn completion_observation_uris(
    outcome: &ProjectAnalysisOutcome,
    sources: &ProjectSourceIndex,
) -> BTreeSet<String> {
    match outcome {
        ProjectAnalysisOutcome::Processed(Ok(result)) => result
            .resources
            .iter()
            .chain(result.targets.iter().flat_map(|target| &target.resources))
            .filter_map(|resource| {
                sources
                    .get(&resource.source_id)
                    .map(|source| source.uri.clone())
                    .or_else(|| {
                        resource
                            .observation
                            .as_ref()
                            .and_then(|candidate| lsp::Url::from_file_path(&candidate.path).ok())
                            .map(|uri| uri.to_string())
                    })
            })
            .collect(),
        ProjectAnalysisOutcome::Processed(Err(error)) => error
            .repair_candidate()
            .and_then(|candidate| lsp::Url::from_file_path(&candidate.path).ok())
            .map(|uri| BTreeSet::from([uri.to_string()]))
            .unwrap_or_default(),
        ProjectAnalysisOutcome::Rejected(_) => BTreeSet::new(),
    }
}

fn adopted_problem(snapshot: &ProjectAnalysisSnapshot, problem: ProjectProblem) -> AdoptedAnalysis {
    let mut published_diagnostic_uris = BTreeSet::from([snapshot.uri.clone()]);
    if let Some(uri) = &problem.document_uri {
        published_diagnostic_uris.insert(uri.clone());
    }
    AdoptedAnalysis {
        primary: None,
        expanded: None,
        format: adocweave::output::formatter::FormatConfig::default(),
        sources: Arc::new(ProjectSourceIndex::default()),
        problem: Some(problem),
        published_diagnostic_uris,
    }
}

fn adopted_project_result(
    snapshot: &ProjectAnalysisSnapshot,
    result: &mut ProjectResult,
    mut sources: ProjectSourceIndex,
) -> AdoptedAnalysis {
    let Some(target) = result.targets.pop() else {
        return adopted_problem(
            snapshot,
            ProjectProblem {
                document_uri: Some(snapshot.uri.clone()),
                range: zero_text_range(),
                code: "project-target-error".to_owned(),
                message: "Project analysis returned no target.".to_owned(),
            },
        );
    };
    let format = *target.config.config.format();
    let mut resource_versions = BTreeMap::new();
    for resource in target.resources.iter().chain(result.resources.iter()) {
        if let Some(source) = sources.get(&resource.source_id) {
            if let Some(version) = source.version {
                resource_versions.insert(source.uri.clone(), version);
            }
            continue;
        }
        let Ok(uri) = lsp::Url::from_file_path(&resource.path) else {
            continue;
        };
        let uri = uri.to_string();
        let existing = sources.source_for_uri(&uri).cloned();
        let text = match &resource.outcome {
            ProjectResourceOutcome::Loaded { source } => Some(source.clone()),
            ProjectResourceOutcome::LoadedOmitted { .. }
            | ProjectResourceOutcome::Present
            | ProjectResourceOutcome::Missing
            | ProjectResourceOutcome::Failed(_) => {
                existing.as_ref().map(|value| value.text.clone())
            }
        };
        let Some(text) = text else {
            continue;
        };
        let version = existing.as_ref().and_then(|value| value.version);
        let generation = existing.as_ref().and_then(|value| value.generation);
        if let Some(version) = version {
            resource_versions.insert(uri.clone(), version);
        }
        sources.insert(
            resource.source_id.clone(),
            ProjectSourceState {
                uri,
                text,
                version,
                generation,
            },
        );
    }
    let sources = Arc::new(sources);
    let analysis = match target.analysis {
        Ok(analysis) => analysis,
        Err(error) => return adopted_problem(snapshot, target_problem(snapshot, error)),
    };
    let (expanded, problem) = match analysis.expanded {
        Ok(expanded) => (
            Some(ExpandedDocumentAnalysis {
                document: Arc::new(expanded.preprocessed.document),
                analysis: Arc::new(expanded.preprocessed.analysis),
                projection: Arc::new(expanded.source_mapping),
                resource_versions,
                local_target_diagnostics: expanded.local_target_diagnostics,
                sources: sources.clone(),
            }),
            None,
        ),
        Err(error) => (
            None,
            Some(expansion_problem(
                snapshot,
                sources.as_ref(),
                &target.resources,
                error,
            )),
        ),
    };
    let mut published_diagnostic_uris = BTreeSet::from([snapshot.uri.clone()]);
    published_diagnostic_uris.extend(target.resources.iter().filter_map(|resource| {
        matches!(
            resource.kind,
            ProjectResourceKind::Primary | ProjectResourceKind::Include
        )
        .then(|| sources.get(&resource.source_id))
        .flatten()
        .map(|source| source.uri.clone())
    }));
    if let Some(uri) = problem
        .as_ref()
        .and_then(|problem| problem.document_uri.as_ref())
    {
        published_diagnostic_uris.insert(uri.clone());
    }
    AdoptedAnalysis {
        primary: Some(analysis.primary),
        expanded,
        format,
        sources,
        problem,
        published_diagnostic_uris,
    }
}

impl Session {
    fn prepare_project_snapshot(&mut self, snapshot: &mut ProjectAnalysisSnapshot) {
        self.pending_project_observations.remove(&snapshot.uri);
        match self.prepare_project_request(snapshot) {
            Ok(request) => snapshot.prepared_request = Some(request),
            Err(problem) => {
                snapshot.project_problem = Some(problem);
            }
        }
    }

    fn prepare_project_request(
        &self,
        snapshot: &ProjectAnalysisSnapshot,
    ) -> Result<PreparedProjectRequest, ProjectProblem> {
        let unsupported = |message: String| ProjectProblem {
            document_uri: Some(snapshot.uri.clone()),
            range: zero_text_range(),
            code: "unsupported-uri".to_owned(),
            message,
        };
        let primary_uri: lsp::Url = snapshot
            .uri
            .parse()
            .map_err(|_| unsupported("The document URI is invalid.".to_owned()))?;
        if primary_uri.scheme() != "file" {
            return Err(unsupported(format!(
                "The URI scheme '{}' is not supported. Only file URIs can be analyzed.",
                primary_uri.scheme()
            )));
        }
        let primary_path = primary_uri.to_file_path().map_err(|_| {
            unsupported("The file URI cannot be converted to a file path.".to_owned())
        })?;
        let (project_root, authority_roots) = self.project_scope(&primary_path);
        let overlay_scope = project_root.clone();
        #[cfg(test)]
        let (project_root, authority_roots, synthetic_test_root) =
            if !project_root.is_dir() || project_root.parent().is_none() {
                let temporary = tempfile::tempdir().map_err(|error| ProjectProblem {
                    document_uri: Some(snapshot.uri.clone()),
                    range: zero_text_range(),
                    code: "project-authority-error".to_owned(),
                    message: error.to_string(),
                })?;
                let root = temporary.path().to_owned();
                (root.clone(), vec![root], Some(temporary))
            } else {
                (project_root, authority_roots, None)
            };
        let authority = ProjectAuthority::open(project_root, authority_roots).map_err(|error| {
            ProjectProblem {
                document_uri: Some(snapshot.uri.clone()),
                range: zero_text_range(),
                code: "project-authority-error".to_owned(),
                message: error.to_string(),
            }
        })?;
        let observation_access = authority.observation_access();

        let mut project_sources = Vec::new();
        let mut source_index = ProjectSourceIndex::default();
        let primary_id = SourceId::new("lsp:source:0");
        #[cfg(test)]
        let primary_path = synthetic_test_root
            .as_ref()
            .map_or(primary_path.clone(), |root| {
                root.path().join(
                    primary_path
                        .strip_prefix("/")
                        .unwrap_or(primary_path.as_path()),
                )
            });
        #[cfg(test)]
        if synthetic_test_root.is_some()
            && let Some(parent) = primary_path.parent()
        {
            std::fs::create_dir_all(parent).map_err(|error| ProjectProblem {
                document_uri: Some(snapshot.uri.clone()),
                range: zero_text_range(),
                code: "project-authority-error".to_owned(),
                message: error.to_string(),
            })?;
        }
        project_sources.push(ProjectSource::new(
            primary_id.clone(),
            primary_path,
            snapshot.document_input.source.clone(),
        ));
        source_index.insert(
            primary_id.clone(),
            ProjectSourceState {
                uri: snapshot.uri.clone(),
                text: snapshot.document_input.source.clone(),
                version: Some(snapshot.document_input.revision.version),
                generation: Some(snapshot.document_input.revision.generation),
            },
        );

        let mut next_source = 1usize;
        for (uri, revision, source) in self.documents.open_project_sources() {
            if uri == snapshot.uri {
                continue;
            }
            let Ok(parsed_uri) = lsp::Url::parse(&uri) else {
                continue;
            };
            if parsed_uri.scheme() != "file" {
                continue;
            }
            let Ok(path) = parsed_uri.to_file_path() else {
                continue;
            };
            if !path.starts_with(&overlay_scope) {
                continue;
            }
            #[cfg(test)]
            let path = synthetic_test_root.as_ref().map_or(path.clone(), |root| {
                root.path()
                    .join(path.strip_prefix("/").unwrap_or(path.as_path()))
            });
            #[cfg(test)]
            if synthetic_test_root.is_some()
                && let Some(parent) = path.parent()
            {
                std::fs::create_dir_all(parent).map_err(|error| ProjectProblem {
                    document_uri: Some(snapshot.uri.clone()),
                    range: zero_text_range(),
                    code: "project-authority-error".to_owned(),
                    message: error.to_string(),
                })?;
            }
            let source_id = SourceId::new(format!("lsp:source:{next_source}"));
            next_source += 1;
            let source: Arc<str> = Arc::from(source);
            project_sources.push(ProjectSource::new(source_id.clone(), path, source.clone()));
            source_index.insert(
                source_id,
                ProjectSourceState {
                    uri,
                    text: source,
                    version: Some(revision.version),
                    generation: Some(revision.generation),
                },
            );
        }
        let overrides = ProjectOverrides {
            enable_lint_rules: self
                .settings
                .enabled_rules
                .iter()
                .filter_map(|code| lint_rule(code).map(|rule| rule.id))
                .collect(),
            ..ProjectOverrides::default()
        };
        Ok(PreparedProjectRequest {
            request: ProjectRequest {
                targets: vec![ProjectTarget::Source(primary_id)],
                sources: project_sources,
                config: ConfigSelection::Discover,
                overrides,
                apply_safe_fixes: false,
                resource_selection: ProjectResourceSelection {
                    local_targets: true,
                    stylesheets: false,
                },
                authority,
                limits: ProjectLimits::default(),
            },
            source_index,
            observation_access,
            #[cfg(test)]
            _synthetic_root: synthetic_test_root,
        })
    }

    fn project_scope(&self, primary: &Path) -> (PathBuf, Vec<PathBuf>) {
        let selected = self
            .workspace_roots
            .values()
            .filter_map(|uri| uri.to_file_path().ok())
            .filter_map(|root| {
                let is_file =
                    std::fs::symlink_metadata(&root).is_ok_and(|metadata| metadata.is_file());
                if is_file {
                    (root == primary)
                        .then(|| root.parent().map(Path::to_owned))
                        .flatten()
                } else {
                    primary.starts_with(&root).then_some(root)
                }
            })
            .max_by_key(|root| root.components().count())
            .or_else(|| primary.parent().map(Path::to_owned))
            .unwrap_or_else(|| primary.to_owned());
        (selected.clone(), vec![selected])
    }

    fn analysis_snapshot_is_current(&self, job: &ProjectAnalysisSnapshot) -> bool {
        self.documents.snapshot_is_current(job)
    }

    fn project_sources_are_current(&self, sources: &ProjectSourceIndex) -> bool {
        sources
            .open_document_revisions()
            .all(|(uri, version, generation)| {
                self.documents.revision_is_current(uri, version, generation)
            })
    }

    #[cfg(test)]
    pub(crate) fn workspace_roots(&self) -> Vec<lsp::Url> {
        self.workspace_roots.values().cloned().collect()
    }

    pub fn initialize(&mut self, params: &lsp::InitializeParams) -> lsp::InitializeResult {
        self.client = ClientProfile::from_capabilities(&params.capabilities);
        self.position_encoding = negotiate_encoding(params);
        let roots: Vec<lsp::Url> = if self.client.workspace_folders {
            params
                .workspace_folders
                .as_ref()
                .into_iter()
                .flatten()
                .map(|folder| folder.uri.clone())
                .collect()
        } else {
            #[allow(deprecated)]
            params
                .root_uri
                .clone()
                .or_else(|| {
                    params
                        .root_path
                        .as_deref()
                        .and_then(|path| lsp::Url::from_directory_path(path).ok())
                })
                .into_iter()
                .collect()
        };
        self.workspace_roots = roots
            .into_iter()
            .map(|uri| (uri.to_string(), uri))
            .collect();
        lsp::InitializeResult {
            capabilities: lsp::ServerCapabilities {
                position_encoding: Some(self.position_encoding.lsp()),
                text_document_sync: Some(lsp::TextDocumentSyncCapability::Options(
                    lsp::TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(lsp::TextDocumentSyncKind::INCREMENTAL),
                        save: Some(
                            lsp::SaveOptions {
                                include_text: Some(true),
                            }
                            .into(),
                        ),
                        ..lsp::TextDocumentSyncOptions::default()
                    },
                )),
                document_symbol_provider: Some(lsp::OneOf::Left(true)),
                code_action_provider: self
                    .client
                    .code_action_quickfix
                    .then_some(lsp::CodeActionProviderCapability::Simple(true)),
                document_formatting_provider: Some(lsp::OneOf::Left(true)),
                hover_provider: Some(lsp::HoverProviderCapability::Simple(true)),
                definition_provider: Some(lsp::OneOf::Left(true)),
                references_provider: Some(lsp::OneOf::Left(true)),
                // Not every position holds a renameable anchor. Clients that
                // support `prepareRename` are told so, and ask before starting
                // a rename that would return no edit. Clients that do not keep
                // the plain declaration and the behaviour they had before.
                rename_provider: Some(if self.client.rename_prepare_support {
                    lsp::OneOf::Right(lsp::RenameOptions {
                        prepare_provider: Some(true),
                        work_done_progress_options: lsp::WorkDoneProgressOptions::default(),
                    })
                } else {
                    lsp::OneOf::Left(true)
                }),
                document_link_provider: Some(lsp::DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: lsp::WorkDoneProgressOptions::default(),
                }),
                semantic_tokens_provider: self.client.semantic_tokens_full.then_some(
                    lsp::SemanticTokensOptions {
                        work_done_progress_options: lsp::WorkDoneProgressOptions::default(),
                        legend: lsp::SemanticTokensLegend {
                            token_types: vec![
                                lsp::SemanticTokenType::STRING,
                                lsp::SemanticTokenType::VARIABLE,
                            ],
                            token_modifiers: Vec::new(),
                        },
                        range: None,
                        full: Some(lsp::SemanticTokensFullOptions::Bool(true)),
                    }
                    .into(),
                ),
                completion_provider: Some(lsp::CompletionOptions {
                    trigger_characters: Some(vec![",".to_owned(), " ".to_owned()]),
                    ..lsp::CompletionOptions::default()
                }),
                workspace: self.client.workspace_folders.then_some(
                    lsp::WorkspaceServerCapabilities {
                        workspace_folders: Some(lsp::WorkspaceFoldersServerCapabilities {
                            supported: Some(true),
                            change_notifications: Some(lsp::OneOf::Left(true)),
                        }),
                        file_operations: None,
                    },
                ),
                ..lsp::ServerCapabilities::default()
            },
            server_info: Some(lsp::ServerInfo {
                name: SERVER_NAME.to_owned(),
                version: Some(VERSION.to_owned()),
            }),
        }
    }

    pub fn begin_open(
        &mut self,
        params: lsp::DidOpenTextDocumentParams,
    ) -> Vec<ProjectAnalysisSnapshot> {
        let document = params.text_document;
        let mut job = self.documents.begin_open_with_options(
            document.uri.to_string(),
            document.version,
            document.text.clone(),
            self.analysis_options(),
        );
        if reject_unsupported_uri(&mut job, &document.uri) {
            return vec![job];
        }
        let affected = self.observing_documents(document.uri.as_str());
        self.prepare_project_snapshot(&mut job);
        let mut jobs = vec![job];
        self.append_dependent_jobs(&affected, document.uri.as_str(), &mut jobs);
        jobs
    }

    pub fn begin_change(
        &mut self,
        params: lsp::DidChangeTextDocumentParams,
    ) -> Result<Vec<ProjectAnalysisSnapshot>, String> {
        let Some(current) = self.documents.get(params.text_document.uri.as_str()) else {
            return Ok(Vec::new());
        };
        if i64::from(params.text_document.version) <= current.document_input.revision.version {
            return Ok(Vec::new());
        }
        let mut source = current.document_input.source.to_string();
        for change in params.content_changes {
            match change.range {
                None => source = change.text,
                Some(range) => {
                    let index = SourceDocument::new(&source).map_err(|error| error.to_string())?;
                    let start = index
                        .position_to_offset(
                            lsp_position_to_core(range.start),
                            self.position_encoding.core(),
                        )
                        .map_err(|error| error.to_string())?
                        .to_usize();
                    let end = index
                        .position_to_offset(
                            lsp_position_to_core(range.end),
                            self.position_encoding.core(),
                        )
                        .map_err(|error| error.to_string())?
                        .to_usize();
                    if start > end {
                        return Err("incremental change range is reversed".to_owned());
                    }
                    source.replace_range(start..end, &change.text);
                }
            }
        }
        let Some(mut job) = self.documents.begin_change(
            params.text_document.uri.as_str(),
            params.text_document.version,
            source.clone(),
        ) else {
            return Ok(Vec::new());
        };
        if reject_unsupported_uri(&mut job, &params.text_document.uri) {
            return Ok(vec![job]);
        }
        let affected = self.observing_documents(params.text_document.uri.as_str());
        self.prepare_project_snapshot(&mut job);
        let mut jobs = vec![job];
        self.append_dependent_jobs(&affected, params.text_document.uri.as_str(), &mut jobs);
        Ok(jobs)
    }

    fn append_dependent_jobs(
        &mut self,
        affected: &std::collections::BTreeSet<String>,
        changed: &str,
        jobs: &mut Vec<ProjectAnalysisSnapshot>,
    ) {
        for uri in affected.iter().filter(|uri| uri.as_str() != changed) {
            let Some(mut job) = self.documents.begin_reanalysis(uri) else {
                continue;
            };
            self.prepare_project_snapshot(&mut job);
            jobs.push(job);
        }
    }

    fn observing_documents(&self, changed: &str) -> BTreeSet<String> {
        let mut documents: BTreeSet<String> = self
            .project_observations
            .iter()
            .filter(|(_, observations)| observations.uris.contains(changed))
            .map(|(uri, _)| uri.clone())
            .collect();
        documents.extend(
            self.pending_project_observations
                .iter()
                .filter(|(_, observations)| observations.uris.contains(changed))
                .map(|(uri, _)| uri.clone()),
        );
        documents
    }

    fn analysis_options(&self) -> adocweave::AnalysisOptions {
        let mut options = adocweave::AnalysisOptions::default();
        for code in &self.settings.enabled_rules {
            let Some(descriptor) = lint_rule(code) else {
                continue;
            };
            options.diagnostics.lint.set_rule(
                descriptor.id,
                RuleSettings {
                    enabled: true,
                    severity: options.diagnostics.lint.rule(descriptor.id).severity,
                },
            );
        }
        options
    }

    pub(crate) fn handle_workspace_files_changed(
        &mut self,
        params: lsp::DidChangeWatchedFilesParams,
    ) -> Vec<ProjectAnalysisSnapshot> {
        let mut changed = BTreeSet::new();
        let mut event_count = 0usize;
        let mut uri_bytes = 0usize;
        for change in params.changes {
            event_count = event_count.saturating_add(1);
            uri_bytes = uri_bytes.saturating_add(change.uri.as_str().len());
            if event_count > MAX_WORKSPACE_WATCH_CHANGES
                || uri_bytes > MAX_WORKSPACE_WATCH_URI_BYTES
            {
                return self.reconfigure_open_documents();
            }
            changed.insert(change.uri.to_string());
        }
        let affected = changed
            .iter()
            .flat_map(|uri| self.observing_documents(uri))
            .collect::<BTreeSet<_>>();
        self.reconfigure_documents(&affected)
    }

    pub fn workspace_folders_changed(
        &mut self,
        params: lsp::DidChangeWorkspaceFoldersParams,
    ) -> Vec<ProjectAnalysisSnapshot> {
        if !self.client.workspace_folders {
            return Vec::new();
        }
        let mut roots = self.workspace_roots.clone();
        for folder in params.event.removed {
            roots.remove(folder.uri.as_str());
        }
        for folder in params.event.added {
            roots.insert(folder.uri.to_string(), folder.uri);
        }
        if roots == self.workspace_roots {
            return Vec::new();
        }
        self.workspace_roots = roots;
        self.reconfigure_open_documents()
    }

    fn reconfigure_open_documents(&mut self) -> Vec<ProjectAnalysisSnapshot> {
        let uris = self
            .documents
            .open_sources()
            .into_iter()
            .map(|(uri, _, _)| uri)
            .collect();
        self.reconfigure_documents(&uris)
    }

    fn reconfigure_documents(&mut self, uris: &BTreeSet<String>) -> Vec<ProjectAnalysisSnapshot> {
        uris.iter()
            .filter_map(|uri| {
                let parsed = lsp::Url::parse(uri).ok()?;
                let mut snapshot = self.documents.reconfigure(uri, self.analysis_options())?;
                if !reject_unsupported_uri(&mut snapshot, &parsed) {
                    self.prepare_project_snapshot(&mut snapshot);
                }
                Some(snapshot)
            })
            .collect()
    }

    pub fn watched_files_registration(&self) -> Option<lsp::RegistrationParams> {
        self.client
            .watched_files_dynamic_registration
            .then(|| lsp::RegistrationParams {
                registrations: vec![lsp::Registration {
                    id: "adocweave-watch-asciidoc".to_owned(),
                    method: "workspace/didChangeWatchedFiles".to_owned(),
                    register_options: Some(
                        serde_json::to_value(lsp::DidChangeWatchedFilesRegistrationOptions {
                            watchers: vec![lsp::FileSystemWatcher {
                                // Include targets may use any extension; the
                                // handler filters unrelated paths before I/O.
                                glob_pattern: lsp::GlobPattern::String("**/*".to_owned()),
                                kind: Some(
                                    lsp::WatchKind::Create
                                        | lsp::WatchKind::Change
                                        | lsp::WatchKind::Delete,
                                ),
                            }],
                        })
                        .expect("watched file registration is serializable"),
                    ),
                }],
            })
    }

    pub(crate) fn project_processing_completed(
        &mut self,
        mut completion: ProjectAnalysisCompletion,
    ) -> ProjectAnalysisAction {
        if !self.analysis_snapshot_is_current(&completion.snapshot) {
            return ProjectAnalysisAction::Ignore;
        }
        // A successful result names every primary and resource it actually
        // observed. Request-wide failures do not carry that complete list, so
        // they conservatively keep and revalidate every captured overlay.
        if let ProjectAnalysisOutcome::Processed(Ok(result)) = &completion.outcome {
            let referenced_sources = result
                .targets
                .iter()
                .map(|target| &target.source_id)
                .chain(result.resources.iter().map(|resource| &resource.source_id))
                .chain(
                    result
                        .targets
                        .iter()
                        .flat_map(|target| &target.resources)
                        .map(|resource| &resource.source_id),
                )
                .cloned()
                .collect();
            completion.source_index.retain(&referenced_sources);
        }
        if !self.project_sources_are_current(&completion.source_index) {
            return self.retry_completion(&completion.snapshot);
        }
        if matches!(completion.outcome, ProjectAnalysisOutcome::Rejected(_)) {
            completion.observations_are_current = Some(true);
            return self.complete_analysis(completion);
        }
        let uris = completion_observation_uris(&completion.outcome, &completion.source_index);
        self.pending_project_observations.insert(
            completion.snapshot.uri.clone(),
            PendingProjectObservations {
                generation: completion.snapshot.document_input.revision.generation,
                uris,
            },
        );
        ProjectAnalysisAction::Validate(Box::new(completion))
    }

    pub(crate) fn complete_analysis(
        &mut self,
        completion: ProjectAnalysisCompletion,
    ) -> ProjectAnalysisAction {
        self.clear_pending_observations(&completion.snapshot);
        if !self.analysis_snapshot_is_current(&completion.snapshot) {
            return ProjectAnalysisAction::Ignore;
        }
        if !self.project_sources_are_current(&completion.source_index) {
            return self.retry_completion(&completion.snapshot);
        }
        if completion.observations_are_current != Some(true) {
            return self.retry_completion(&completion.snapshot);
        }

        let observed_uris =
            completion_observation_uris(&completion.outcome, &completion.source_index);
        let records_observations =
            matches!(completion.outcome, ProjectAnalysisOutcome::Processed(_));
        let adopted = match completion.outcome {
            ProjectAnalysisOutcome::Processed(Ok(mut result)) => {
                adopted_project_result(&completion.snapshot, &mut result, completion.source_index)
            }
            ProjectAnalysisOutcome::Processed(Err(error)) => {
                if matches!(error, ProjectError::Cancelled) {
                    return ProjectAnalysisAction::Ignore;
                }
                adopted_problem(
                    &completion.snapshot,
                    project_error_problem(&completion.snapshot.uri, &error),
                )
            }
            ProjectAnalysisOutcome::Rejected(problem) => {
                adopted_problem(&completion.snapshot, problem)
            }
        };
        let mut diagnostic_uris = completion
            .snapshot
            .previously_published_diagnostic_uris
            .clone();
        diagnostic_uris.extend(adopted.published_diagnostic_uris.iter().cloned());
        if self
            .documents
            .complete_analysis(&completion.snapshot, adopted)
            != Adoption::Adopted
        {
            return ProjectAnalysisAction::Ignore;
        }
        if records_observations {
            self.project_observations.insert(
                completion.snapshot.uri.clone(),
                ProjectObservations {
                    uris: observed_uris,
                },
            );
        } else {
            self.project_observations.remove(&completion.snapshot.uri);
        }
        ProjectAnalysisAction::Publish {
            snapshot: completion.snapshot,
            diagnostic_uris: diagnostic_uris.into_iter().collect(),
        }
    }

    fn retry_completion(&mut self, snapshot: &ProjectAnalysisSnapshot) -> ProjectAnalysisAction {
        self.retry_project_analysis(snapshot)
            .map_or(ProjectAnalysisAction::Ignore, ProjectAnalysisAction::Retry)
    }

    fn clear_pending_observations(&mut self, snapshot: &ProjectAnalysisSnapshot) {
        if self
            .pending_project_observations
            .get(&snapshot.uri)
            .is_some_and(|pending| {
                pending.generation == snapshot.document_input.revision.generation
            })
        {
            self.pending_project_observations.remove(&snapshot.uri);
        }
    }

    pub fn retry_project_analysis(
        &mut self,
        job: &ProjectAnalysisSnapshot,
    ) -> Option<ProjectAnalysisSnapshot> {
        if !self.analysis_snapshot_is_current(job) {
            return None;
        }
        self.rebuild_current_project_job(job)
    }

    fn rebuild_current_project_job(
        &mut self,
        job: &ProjectAnalysisSnapshot,
    ) -> Option<ProjectAnalysisSnapshot> {
        let mut retry = self
            .documents
            .reconfigure(&job.uri, self.analysis_options())?;
        self.prepare_project_snapshot(&mut retry);
        Some(retry)
    }

    pub fn close(&mut self, uri: &lsp::Url) -> SessionCloseOutcome {
        let affected = self.observing_documents(uri.as_str());
        self.pending_project_observations.remove(uri.as_str());
        self.project_observations.remove(uri.as_str());
        let diagnostic_uris = self.documents.published_diagnostic_uris(uri.as_str());
        let closed = self.documents.close(uri.as_str());
        let mut jobs = Vec::new();
        self.append_dependent_jobs(&affected, uri.as_str(), &mut jobs);
        SessionCloseOutcome {
            closed,
            reanalysis_jobs: jobs,
            diagnostic_uris,
        }
    }

    pub fn shutdown(&mut self) {
        self.documents.cancel_all();
        self.pending_project_observations.clear();
    }

    pub fn document_cancellation(
        &self,
        uri: &lsp::Url,
    ) -> Option<Arc<adocweave::CancellationToken>> {
        self.documents.cancellation(uri.as_str())
    }

    pub fn update_configuration(
        &mut self,
        settings: serde_json::Value,
    ) -> Result<Vec<ProjectAnalysisSnapshot>, String> {
        let settings = settings.get("adocweave").cloned().unwrap_or(settings);
        let mut settings: ServerSettings =
            serde_json::from_value(settings).map_err(|error| error.to_string())?;
        settings.debounce_ms = settings.debounce_ms.min(1_000);
        for code in &settings.enabled_rules {
            let descriptor =
                lint_rule(code).ok_or_else(|| format!("unknown diagnostic rule: {code}"))?;
            if descriptor.default_enabled {
                return Err(format!(
                    "diagnostic rule cannot be enabled explicitly: {code}"
                ));
            }
        }
        let diagnostics_changed = self.settings.enabled_rules != settings.enabled_rules;
        self.settings = settings;
        if !diagnostics_changed {
            return Ok(Vec::new());
        }
        Ok(self.reconfigure_open_documents())
    }

    pub const fn debounce_ms(&self) -> u64 {
        self.settings.debounce_ms
    }

    pub fn diagnostics(&self, uri: &lsp::Url) -> Result<lsp::PublishDiagnosticsParams, String> {
        let document = self.documents.get(uri.as_str());
        let source = document
            .map(|document| document.document_input.source.as_ref())
            .or_else(|| self.documents.adopted_source(uri.as_str()));
        let Some(source) = source else {
            return Ok(lsp::PublishDiagnosticsParams::new(
                uri.clone(),
                Vec::new(),
                None,
            ));
        };
        let source_document = SourceDocument::new(source).map_err(|error| error.to_string())?;
        let version = self
            .client
            .diagnostic_version
            .then(|| {
                document.map(|document| revision_version_i32(&document.document_input.revision))
            })
            .flatten();
        let mut diagnostics = document
            .and_then(|document| document.view.as_ref().map(|view| view.primary.as_ref()))
            .iter()
            .flat_map(|analysis| analysis.diagnostics().iter())
            .map(|diagnostic| {
                crate::diagnostics::analysis_diagnostic(
                    uri,
                    diagnostic,
                    &source_document,
                    self.position_encoding,
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        for expanded in self.documents.expanded_analyses() {
            let current_version = expanded.resource_versions.get(uri.as_str()).copied();
            let is_root = expanded.analysis.source_id().is_some_and(|source_id| {
                expanded.uri_for_source_id(source_id) == Some(uri.as_str())
            });
            if current_version != document.map(|document| document.document_input.revision.version)
            {
                continue;
            }
            if !is_root {
                // Reading the map here is intentional: the diagnostics and source map belong to
                // the same adopted result and must never be mixed with a later project input.
                let _source_map = expanded.document.source_map();
                for projected in &expanded.projection.diagnostics {
                    for origin in &projected.origins {
                        if origin.source_id.as_ref().is_none_or(|source_id| {
                            expanded.uri_for_source_id(source_id) != Some(uri.as_str())
                        }) {
                            continue;
                        }
                        diagnostics.push(crate::diagnostics::projected_diagnostic(
                            origin.range.text_range(),
                            &projected.diagnostic,
                            &source_document,
                            self.position_encoding,
                        )?);
                    }
                }
            }
            for local_target in &expanded.local_target_diagnostics {
                if expanded.uri_for_source_id(&local_target.source_id) != Some(uri.as_str()) {
                    continue;
                }
                diagnostics.push(crate::diagnostics::projected_diagnostic(
                    local_target.diagnostic.range,
                    &local_target.diagnostic,
                    &source_document,
                    self.position_encoding,
                )?);
            }
        }
        for problem in self.documents.project_problems() {
            if problem.document_uri.as_deref() != Some(uri.as_str()) {
                continue;
            }
            diagnostics.push(crate::diagnostics::project_problem(
                problem.range,
                &problem.code,
                &problem.message,
                &source_document,
                self.position_encoding,
            )?);
        }
        crate::diagnostics::canonicalize(&mut diagnostics);
        Ok(lsp::PublishDiagnosticsParams::new(
            uri.clone(),
            diagnostics,
            version,
        ))
    }

    pub fn document_symbols_cancellable(
        &self,
        uri: &lsp::Url,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<lsp::DocumentSymbolResponse>> {
        cancellation.check_now()?;
        let presentation = if self.client.hierarchical_document_symbols {
            SymbolPresentation::Hierarchical
        } else {
            SymbolPresentation::Flat
        };
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(Some(match presentation {
                SymbolPresentation::Hierarchical => lsp::DocumentSymbolResponse::Nested(Vec::new()),
                SymbolPresentation::Flat => lsp::DocumentSymbolResponse::Flat(Vec::new()),
            }));
        };
        let response = crate::document_symbols::symbols(
            &document.analysis,
            uri,
            self.position_encoding,
            presentation,
        )?;
        cancellation.check_now()?;
        Ok(Some(response))
    }

    pub fn code_actions_cancellable(
        &self,
        uri: &lsp::Url,
        range: lsp::Range,
        context: &lsp::CodeActionContext,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<Vec<lsp::CodeActionOrCommand>>> {
        cancellation.check_now()?;
        if !self.client.code_action_quickfix
            || !code_action_kind_requested(context.only.as_deref(), &lsp::CodeActionKind::QUICKFIX)
        {
            return Ok(Some(Vec::new()));
        }
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(Some(Vec::new()));
        };
        let actions = crate::diagnostics::quick_fixes(
            uri,
            revision_version_i32(&document.revision),
            &document.analysis,
            range,
            self.position_encoding,
            QuickFixCapabilities {
                versioned_document_changes: self.client.versioned_document_changes,
                is_preferred: self.client.code_action_is_preferred,
            },
        )?;
        cancellation.check_now()?;
        Ok(Some(actions))
    }

    pub fn formatting_cancellable(
        &self,
        uri: &lsp::Url,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<Vec<lsp::TextEdit>>> {
        cancellation.check_now()?;
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(Some(Vec::new()));
        };
        editing::formatting(
            &document.analysis,
            &document.format,
            self.position_encoding,
            cancellation,
        )
        .map(Some)
    }

    pub fn hover_cancellable(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<lsp::Hover>> {
        cancellation.check_now()?;
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(None);
        };
        let offset = request_offset(
            document.analysis.source_document(),
            position,
            self.position_encoding,
        )?;
        crate::hover::hover(
            &document.analysis,
            uri,
            offset,
            self.documents.expanded_analyses(),
            self.position_encoding,
            self.client.hover,
        )
        .map_err(Into::into)
    }

    pub fn completion_cancellable(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<lsp::CompletionResponse>> {
        cancellation.check_now()?;
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(Some(presentation::empty_completion()));
        };
        let expanded_analyses = self.documents.expanded_analyses().collect::<Vec<_>>();
        let response = presentation::completion(
            &document.analysis,
            &expanded_analyses,
            uri,
            position,
            self.position_encoding,
        )?;
        cancellation.check_now()?;
        Ok(Some(response))
    }

    pub fn definition_cancellable(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<lsp::GotoDefinitionResponse>> {
        cancellation.check_now()?;
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(None);
        };
        let snapshots = self.documents.snapshots();
        let expanded_analyses = self.documents.expanded_analyses().collect::<Vec<_>>();
        let source_document = |source_uri: &lsp::Url| self.source_document(source_uri);
        let input = NavigationInput {
            document: &document,
            snapshots: &snapshots,
            expanded_analyses: &expanded_analyses,
            encoding: self.position_encoding,
            source_document: &source_document,
        };
        match navigation::definition(&input, uri, position, cancellation)? {
            navigation::Definition::Resolved(response) => Ok(response),
            navigation::Definition::Unresolved => Ok(None),
        }
    }

    pub fn references_cancellable(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
        include_declaration: bool,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<Vec<lsp::Location>>> {
        cancellation.check_now()?;
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(Some(Vec::new()));
        };
        let snapshots = self.documents.snapshots();
        let expanded_analyses = self.documents.expanded_analyses().collect::<Vec<_>>();
        let source_document = |source_uri: &lsp::Url| self.source_document(source_uri);
        let input = NavigationInput {
            document: &document,
            snapshots: &snapshots,
            expanded_analyses: &expanded_analyses,
            encoding: self.position_encoding,
            source_document: &source_document,
        };
        let result =
            navigation::references(&input, uri, position, include_declaration, cancellation)?;
        Ok(Some(result.fallback))
    }

    /// Answers whether a rename may start at this position.
    ///
    /// `None` tells the client the position cannot be renamed, so it reports
    /// that instead of opening a rename the server would answer with nothing.
    /// The anchor's current text becomes the placeholder the editor offers.
    pub fn prepare_rename_cancellable(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<lsp::PrepareRenameResponse>> {
        cancellation.check_now()?;
        if !self.client.rename_prepare_support {
            return Ok(None);
        }
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(None);
        };
        let Some(target) =
            editing::renameable_anchor(&document.analysis, position, self.position_encoding)?
        else {
            return Ok(None);
        };
        if self
            .rename_locations_cancellable(&document, uri, position, &target, cancellation)?
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(lsp::PrepareRenameResponse::RangeWithPlaceholder {
            range: target.range,
            placeholder: target.placeholder,
        }))
    }

    pub fn rename_cancellable(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
        new_name: &str,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<lsp::WorkspaceEdit>> {
        cancellation.check_now()?;
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(None);
        };
        let Some(target) = editing::rename_target(
            &document.analysis,
            position,
            new_name,
            self.position_encoding,
        )?
        else {
            return Ok(None);
        };
        let Some(locations) =
            self.rename_locations_cancellable(&document, uri, position, &target, cancellation)?
        else {
            return Ok(None);
        };
        cancellation.check_now()?;
        Ok(editing::rename_edit(locations, new_name))
    }

    fn rename_locations_cancellable(
        &self,
        document: &DocumentSnapshot,
        uri: &lsp::Url,
        position: lsp::Position,
        target: &editing::RenameTarget,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<Vec<lsp::Location>>> {
        let snapshots = self.documents.snapshots();
        let expanded_analyses = self.documents.expanded_analyses().collect::<Vec<_>>();
        let source_document = |source_uri: &lsp::Url| self.source_document(source_uri);
        let input = NavigationInput {
            document,
            snapshots: &snapshots,
            expanded_analyses: &expanded_analyses,
            encoding: self.position_encoding,
            source_document: &source_document,
        };
        let references = navigation::references(&input, uri, position, false, cancellation)?;
        if !references.anchor_occurrences_are_authored {
            return Ok(None);
        }
        let mut locations = references.fallback;
        locations.push(lsp::Location::new(uri.clone(), target.range));
        Ok(Some(locations))
    }

    pub fn document_links_cancellable(
        &self,
        uri: &lsp::Url,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<Vec<lsp::DocumentLink>>> {
        cancellation.check_now()?;
        let Some(document) = self.documents.snapshot(uri.as_str()) else {
            return Ok(Some(Vec::new()));
        };
        let snapshots = self.documents.snapshots();
        let expanded_analyses = self.documents.expanded_analyses().collect::<Vec<_>>();
        let source_document = |source_uri: &lsp::Url| self.source_document(source_uri);
        let input = NavigationInput {
            document: &document,
            snapshots: &snapshots,
            expanded_analyses: &expanded_analyses,
            encoding: self.position_encoding,
            source_document: &source_document,
        };
        let tooltips = self.client.document_link_tooltip;
        let links = navigation::document_links(&input, uri, tooltips, cancellation)?;
        Ok(Some(links.finish(cancellation)?))
    }

    fn source_document(&self, uri: &lsp::Url) -> Result<SourceDocument, String> {
        let source = self
            .documents
            .get(uri.as_str())
            .map(|document| document.document_input.source.as_ref())
            .or_else(|| self.documents.adopted_source(uri.as_str()))
            .ok_or_else(|| format!("projected source is missing: {uri}"))?;
        SourceDocument::new(source).map_err(|error| error.to_string())
    }

    pub fn semantic_tokens_cancellable(
        &self,
        uri: &lsp::Url,
        cancellation: &QueryCancellation,
    ) -> QueryResult<Option<lsp::SemanticTokensResult>> {
        cancellation.check_now()?;
        if !self.client.semantic_tokens_full {
            return Ok(None);
        }
        let document = self.documents.snapshot(uri.as_str());
        crate::semantic_tokens::response(
            document.as_ref().map(|document| document.analysis.as_ref()),
            self.position_encoding,
            cancellation,
        )
        .map(Some)
    }

    #[cfg(test)]
    fn test_query<T>(result: QueryResult<T>) -> Result<T, String> {
        result.map_err(|error| error.to_string())
    }

    #[cfg(test)]
    pub fn document_symbols(
        &self,
        uri: &lsp::Url,
    ) -> Result<Option<lsp::DocumentSymbolResponse>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.document_symbols_cancellable(uri, &cancellation))
    }

    #[cfg(test)]
    pub fn code_actions(
        &self,
        uri: &lsp::Url,
        range: lsp::Range,
        context: &lsp::CodeActionContext,
    ) -> Result<Option<Vec<lsp::CodeActionOrCommand>>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.code_actions_cancellable(uri, range, context, &cancellation))
    }

    #[cfg(test)]
    pub fn formatting(&self, uri: &lsp::Url) -> Result<Option<Vec<lsp::TextEdit>>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.formatting_cancellable(uri, &cancellation))
    }

    #[cfg(test)]
    pub fn hover(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
    ) -> Result<Option<lsp::Hover>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.hover_cancellable(uri, position, &cancellation))
    }

    #[cfg(test)]
    pub fn completion(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
    ) -> Result<Option<lsp::CompletionResponse>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.completion_cancellable(uri, position, &cancellation))
    }

    #[cfg(test)]
    pub fn definition(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
    ) -> Result<Option<lsp::GotoDefinitionResponse>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.definition_cancellable(uri, position, &cancellation))
    }

    #[cfg(test)]
    pub fn references(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
        include_declaration: bool,
    ) -> Result<Option<Vec<lsp::Location>>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.references_cancellable(
            uri,
            position,
            include_declaration,
            &cancellation,
        ))
    }

    #[cfg(test)]
    pub fn prepare_rename(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
    ) -> Result<Option<lsp::PrepareRenameResponse>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.prepare_rename_cancellable(uri, position, &cancellation))
    }

    #[cfg(test)]
    pub fn rename(
        &self,
        uri: &lsp::Url,
        position: lsp::Position,
        new_name: &str,
    ) -> Result<Option<lsp::WorkspaceEdit>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.rename_cancellable(uri, position, new_name, &cancellation))
    }

    #[cfg(test)]
    pub fn document_links(&self, uri: &lsp::Url) -> Result<Option<Vec<lsp::DocumentLink>>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.document_links_cancellable(uri, &cancellation))
    }

    #[cfg(test)]
    pub fn semantic_tokens(
        &self,
        uri: &lsp::Url,
    ) -> Result<Option<lsp::SemanticTokensResult>, String> {
        let cancellation = crate::cancellation::test_cancellation();
        Self::test_query(self.semantic_tokens_cancellable(uri, &cancellation))
    }
}

fn code_action_kind_requested(
    only: Option<&[lsp::CodeActionKind]>,
    offered: &lsp::CodeActionKind,
) -> bool {
    only.is_none_or(|requested| {
        requested.iter().any(|kind| {
            offered == kind
                || offered
                    .as_str()
                    .strip_prefix(kind.as_str())
                    .is_some_and(|suffix| suffix.starts_with('.'))
        })
    })
}

fn revision_version_i32(revision: &adocweave::DocumentRevision) -> i32 {
    i32::try_from(revision.version).expect("LSP document versions originate as i32")
}
