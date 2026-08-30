//! Runtime-independent language features over owned document analyses.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use adocweave::CancellationCheck;
use adocweave::SourceId;
use adocweave::output::diagnostics::{RuleSettings, lint_rule};
#[cfg(test)]
use adocweave::output::formatter;
use adocweave::resolution::ReferenceKey;
use adocweave::text::SourceDocument;
use adocweave_project::{
    ConfigSelection, ProjectAuthority, ProjectExpansionError, ProjectLimits,
    ProjectObservationAccess, ProjectOverrides, ProjectRequest, ProjectResourceErrorCode,
    ProjectResourceKind, ProjectResourceOutcome, ProjectResourceResult, ProjectResourceSelection,
    ProjectResult, ProjectSource, ProjectTarget, ProjectTargetError,
};
use adocweave_workspace::{ResourceId, ResourceLayer};
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
    Adoption, AnalysisJob, DocumentSnapshot, ExpandedDocumentAnalysis, PreparedProjectRequest,
    ProjectAdoption, ProjectProblem, ProjectSourceIndex, ProjectSourceState,
};
use crate::workspace::{WatchedFileKind, WorkspaceResources, WorkspaceScanNotice};
use crate::workspace_scan::{
    WorkspaceScanCoordinator, WorkspaceScanRecovery, WorkspaceScanStart, WorkspaceScanTransition,
    WorkspaceScanned,
};
use crate::{SERVER_NAME, VERSION};

const MAX_WORKSPACE_WATCH_ERRORS: usize = 128;
const MAX_WORKSPACE_WATCH_ERROR_BYTES: usize = 64 * 1024;
const MAX_WORKSPACE_WATCH_CHANGES: usize = 10_000;
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

/// A completed read of the workspace roots, waiting to be installed.
///
/// Carries no borrow of the service, so it can be produced on a worker and
/// handed back to the event loop.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct WorkspaceScan {
    loaded: crate::workspace::LoadedRoots,
}

pub trait HostReferenceIndex: Send + Sync {
    fn definition(&self, request: &HostReferenceRequest) -> Result<Option<lsp::Location>, String>;

    /// Returns reference occurrences using ranges that can replace the referenced symbol.
    ///
    /// For an anchor, each non-declaration occurrence is only the authored
    /// identifier, excluding a document locator and `#`. When
    /// `include_declaration` is false, the result must not contain the declaration.
    fn references(
        &self,
        request: &HostReferenceRequest,
        include_declaration: bool,
    ) -> Result<Option<Vec<lsp::Location>>, String>;

    fn is_complete(&self) -> bool;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostReferenceRequest {
    pub source: lsp::Url,
    pub source_version: i32,
    pub source_generation: u64,
    pub target: ReferenceKey,
    pub encoding: PositionEncoding,
}

#[derive(Debug, Default)]
pub struct NoHostReferenceIndex;

impl HostReferenceIndex for NoHostReferenceIndex {
    fn definition(&self, _request: &HostReferenceRequest) -> Result<Option<lsp::Location>, String> {
        Ok(None)
    }

    fn references(
        &self,
        _request: &HostReferenceRequest,
        _include_declaration: bool,
    ) -> Result<Option<Vec<lsp::Location>>, String> {
        Ok(None)
    }

    fn is_complete(&self) -> bool {
        false
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
    input_revision: u64,
    workspace_input_epoch: u64,
    client: ClientProfile,
    settings: ServerSettings,
    host_index: Arc<dyn HostReferenceIndex>,
    workspace: WorkspaceResources,
    workspace_roots: std::collections::BTreeMap<String, lsp::Url>,
    workspace_error: Option<WorkspaceEpochError>,
    /// Incomplete-scan reasons whose notification period is still active.
    ///
    /// A failed scan does not end a period because it establishes neither a
    /// complete result nor a new set of incomplete reasons.
    active_scan_notices: std::collections::BTreeSet<WorkspaceScanNotice>,
    workspace_watch_errors: std::collections::BTreeMap<String, String>,
    workspace_watch_error_bytes: usize,
    workspace_watch_errors_overflowed: bool,
    workspace_watch_recovery_required: bool,
    workspace_watch_error_epoch: u64,
    workspace_scans: Arc<Mutex<WorkspaceScanCoordinator>>,
    workspace_input_status: WorkspaceInputStatus,
}

pub(crate) struct SessionCloseOutcome {
    pub closed: bool,
    pub reanalysis_jobs: Vec<AnalysisJob>,
    pub diagnostic_uris: std::collections::BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum WorkspaceInputStatus {
    #[default]
    Ready,
    Rebuilding,
}

#[derive(Clone, Debug)]
struct WorkspaceEpochError {
    epoch: u64,
    message: String,
}

pub(crate) struct WorkspaceFileChanges {
    pub(crate) jobs: Vec<AnalysisJob>,
    pub(crate) journal: Vec<lsp::FileEvent>,
    /// Whether `journal` can reproduce every change from this notification
    /// after an in-flight workspace snapshot is installed.
    pub(crate) replay_complete: bool,
    pub(crate) recovery_required: bool,
}

pub(crate) struct WorkspaceFileEventOutcome {
    pub(crate) jobs: Vec<AnalysisJob>,
    pub(crate) recovery_generation: Option<u64>,
    pub(crate) rebuild: Option<WorkspaceScanStart>,
    pub(crate) cancel_recovery_timer: bool,
}

#[allow(dead_code)]
pub(crate) struct WorkspaceScanApplication {
    pub(crate) jobs: Vec<AnalysisJob>,
    pub(crate) installed: bool,
    pub(crate) structural_rebuild_pending: bool,
    pub(crate) notices: Vec<WorkspaceScanNotice>,
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("documents", &self.documents)
            .field("position_encoding", &self.position_encoding)
            .field("input_revision", &self.input_revision)
            .field("workspace_input_epoch", &self.workspace_input_epoch)
            .field("client", &self.client)
            .field("settings", &self.settings)
            .field("workspace_input_status", &self.workspace_input_status)
            .field("has_complete_host_index", &self.host_index.is_complete())
            .finish()
    }
}

impl Default for Session {
    fn default() -> Self {
        Self {
            documents: DocumentStore::default(),
            position_encoding: PositionEncoding::Utf16,
            input_revision: 0,
            workspace_input_epoch: 0,
            client: ClientProfile::default(),
            settings: ServerSettings::default(),
            host_index: Arc::new(NoHostReferenceIndex),
            workspace: WorkspaceResources::default(),
            workspace_roots: std::collections::BTreeMap::new(),
            workspace_error: None,
            active_scan_notices: std::collections::BTreeSet::new(),
            workspace_watch_errors: std::collections::BTreeMap::new(),
            workspace_watch_error_bytes: 0,
            workspace_watch_errors_overflowed: false,
            workspace_watch_recovery_required: false,
            workspace_watch_error_epoch: 0,
            workspace_scans: Arc::new(Mutex::new(WorkspaceScanCoordinator::default())),
            workspace_input_status: WorkspaceInputStatus::Ready,
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

fn attach_project_context(
    job: &mut AnalysisJob,
    context: Result<crate::workspace::ProjectAnalysisContext, String>,
) {
    match context {
        Ok(context) => job.project_context = Some(context),
        Err(message) => {
            job.project_problem = Some(ProjectProblem {
                document_uri: Some(job.uri.clone()),
                range: adocweave::text::TextRange::new(
                    adocweave::text::TextSize::ZERO,
                    adocweave::text::TextSize::ZERO,
                )
                .expect("zero range"),
                code: "workspace-input-error".to_owned(),
                message,
            });
        }
    }
}

fn reject_unsupported_uri(job: &mut AnalysisJob, uri: &lsp::Url) -> bool {
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

fn parse_open_sources(sources: &[(String, i32, String)]) -> Vec<(lsp::Url, i64, Arc<str>)> {
    sources
        .iter()
        .filter_map(|(uri, version, source)| {
            let uri: lsp::Url = uri.parse().ok()?;
            if uri.scheme() != "file" {
                return None;
            }
            Some((uri, i64::from(*version), Arc::<str>::from(source.as_str())))
        })
        .collect()
}

fn zero_text_range() -> adocweave::text::TextRange {
    adocweave::text::TextRange::new(
        adocweave::text::TextSize::ZERO,
        adocweave::text::TextSize::ZERO,
    )
    .expect("zero range")
}

fn target_problem(job: &AnalysisJob, error: ProjectTargetError) -> ProjectProblem {
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

fn expansion_problem(
    job: &AnalysisJob,
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
    result: &ProjectResult,
    access: &ProjectObservationAccess,
    cancellation: &dyn CancellationCheck,
) -> bool {
    let Ok(mut session) = access.session() else {
        return false;
    };
    for candidate in result
        .resources
        .iter()
        .chain(result.targets.iter().flat_map(|target| &target.resources))
        .filter_map(|resource| resource.observation.as_ref())
    {
        if cancellation.is_cancelled()
            || session.observe(&candidate.path, candidate.kind) != candidate.observation
        {
            return false;
        }
    }
    true
}

impl Session {
    fn prepare_project_job(
        &mut self,
        job: &mut AnalysisJob,
        context: Result<crate::workspace::ProjectAnalysisContext, String>,
    ) {
        attach_project_context(job, context);
        if job.project_problem.is_some() {
            self.documents.reject_project_input(job);
            return;
        }
        match self.prepare_project_request(job) {
            Ok(request) => job.prepared_request = Some(request),
            Err(problem) => {
                job.project_problem = Some(problem);
                self.documents.reject_project_input(job);
            }
        }
    }

    fn prepare_project_request(
        &self,
        job: &AnalysisJob,
    ) -> Result<PreparedProjectRequest, ProjectProblem> {
        let unsupported = |message: String| ProjectProblem {
            document_uri: Some(job.uri.clone()),
            range: zero_text_range(),
            code: "unsupported-uri".to_owned(),
            message,
        };
        let primary_uri: lsp::Url = job
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
        let input = job.project_context.as_ref().ok_or_else(|| ProjectProblem {
            document_uri: Some(job.uri.clone()),
            range: zero_text_range(),
            code: "project-input-error".to_owned(),
            message: "Project context is unavailable.".to_owned(),
        })?;
        #[cfg(test)]
        let (project_root, authority_roots, synthetic_test_root) =
            if !input.project_root.is_dir() || input.project_root.parent().is_none() {
                let temporary = tempfile::tempdir().map_err(|error| ProjectProblem {
                    document_uri: Some(job.uri.clone()),
                    range: zero_text_range(),
                    code: "project-authority-error".to_owned(),
                    message: error.to_string(),
                })?;
                let root = temporary.path().to_owned();
                (root.clone(), vec![root], Some(temporary))
            } else {
                (
                    input.project_root.clone(),
                    input.authority_roots.clone(),
                    None,
                )
            };
        #[cfg(not(test))]
        let (project_root, authority_roots) =
            (input.project_root.clone(), input.authority_roots.clone());
        let authority = ProjectAuthority::open(project_root, authority_roots).map_err(|error| {
            ProjectProblem {
                document_uri: Some(job.uri.clone()),
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
                document_uri: Some(job.uri.clone()),
                range: zero_text_range(),
                code: "project-authority-error".to_owned(),
                message: error.to_string(),
            })?;
        }
        project_sources.push(ProjectSource::new(
            primary_id.clone(),
            primary_path,
            job.document_input.source.clone(),
        ));
        source_index.insert(
            primary_id.clone(),
            ProjectSourceState {
                uri: job.uri.clone(),
                text: job.document_input.source.clone(),
                version: Some(job.document_input.revision.version),
            },
        );

        let mut next_source = 1usize;
        for (uri, version, source) in self.documents.open_sources() {
            if uri == job.uri {
                continue;
            }
            let Ok(resource_id) = ResourceId::new(uri.clone()) else {
                continue;
            };
            if input.resource_snapshot.get(&resource_id).is_none() {
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
                    document_uri: Some(job.uri.clone()),
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
                    version: Some(i64::from(version)),
                },
            );
        }
        for (resource_id, resource) in input.resource_snapshot.resources() {
            if resource.layer() == ResourceLayer::Disk {
                source_index
                    .record_version(resource_id.as_str().to_owned(), resource.revision().get());
            }
        }
        let config = ConfigSelection::Resolved(input.project_config.clone());
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
                config,
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

    fn advance_input_revision(&mut self) {
        self.input_revision = self
            .input_revision
            .checked_add(1)
            .expect("Language Server session input revision exhausted");
    }

    fn advance_workspace_input_epoch(&mut self) {
        self.workspace_input_epoch = self
            .workspace_input_epoch
            .checked_add(1)
            .expect("Language Server workspace input epoch exhausted");
    }

    fn current_workspace_error(&self) -> Option<&str> {
        self.workspace_error
            .as_ref()
            .filter(|error| error.epoch == self.workspace_input_epoch)
            .map(|error| error.message.as_str())
    }

    fn set_workspace_error(&mut self, message: String) {
        self.workspace_error = Some(WorkspaceEpochError {
            epoch: self.workspace_input_epoch,
            message,
        });
    }

    fn analysis_job_is_current(&self, job: &AnalysisJob) -> bool {
        self.workspace_input_status == WorkspaceInputStatus::Ready
            && self.documents.job_is_current(job)
    }

    fn invalidate_all_document_inputs(&mut self) {
        self.advance_input_revision();
        self.documents.invalidate_all_inputs(self.input_revision);
        self.workspace_input_status = WorkspaceInputStatus::Rebuilding;
    }

    fn begin_workspace_rebuild(&mut self) {
        self.advance_workspace_input_epoch();
        self.invalidate_all_document_inputs();
    }

    #[cfg(test)]
    pub(crate) const fn input_revision(&self) -> u64 {
        self.input_revision
    }

    #[cfg(test)]
    pub(crate) fn set_input_revision_for_test(&mut self, revision: u64) {
        self.input_revision = revision;
    }

    #[cfg(test)]
    pub(crate) const fn workspace_input_epoch(&self) -> u64 {
        self.workspace_input_epoch
    }

    #[cfg(test)]
    pub(crate) fn set_workspace_input_epoch_for_test(&mut self, epoch: u64) {
        self.workspace_input_epoch = epoch;
    }

    #[cfg(test)]
    pub(crate) fn workspace_roots(&self) -> Vec<lsp::Url> {
        self.workspace_roots.values().cloned().collect()
    }

    pub fn with_host_index(host_index: Arc<dyn HostReferenceIndex>) -> Self {
        Self {
            host_index,
            ..Self::default()
        }
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
        let roots = self.workspace_roots.values().cloned().collect::<Vec<_>>();
        let configured = self.workspace.configure_roots(&roots, &[]);
        self.advance_workspace_input_epoch();
        self.workspace_error = None;
        if let Err(error) = configured {
            self.set_workspace_error(error);
        }
        self.clear_workspace_watch_errors();
        self.advance_input_revision();
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

    pub fn begin_open(&mut self, params: lsp::DidOpenTextDocumentParams) -> Vec<AnalysisJob> {
        let document = params.text_document;
        self.advance_input_revision();
        let mut job = self.documents.begin_open_with_options(
            document.uri.to_string(),
            document.version,
            document.text.clone(),
            self.analysis_options_for(None),
            self.input_revision,
        );
        if reject_unsupported_uri(&mut job, &document.uri) {
            self.documents.reject_project_input(&job);
            return vec![job];
        }
        if self.workspace_input_status == WorkspaceInputStatus::Rebuilding {
            self.documents.mark_project_input_pending(&job);
            return Vec::new();
        }
        if self.workspace_roots.is_empty() {
            return self.configure_open_workspace();
        }
        let affected = match self.workspace.upsert_open(
            document.uri.clone(),
            i64::from(document.version),
            document.text,
        ) {
            Ok(affected) => affected,
            Err(error) => {
                self.prepare_project_job(&mut job, Err(error));
                return vec![job];
            }
        };
        let project_context = self.workspace.project_analysis_context(&document.uri);
        let options = self.analysis_options_for(project_context.as_ref().ok());
        self.documents.update_job_options(&mut job, options);
        self.prepare_project_job(&mut job, project_context);
        let mut jobs = vec![job];
        self.append_dependent_jobs(&affected, document.uri.as_str(), &mut jobs);
        jobs
    }

    pub fn begin_change(
        &mut self,
        params: lsp::DidChangeTextDocumentParams,
    ) -> Result<Vec<AnalysisJob>, String> {
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
        self.advance_input_revision();
        let Some(mut job) = self.documents.begin_change(
            params.text_document.uri.as_str(),
            params.text_document.version,
            source.clone(),
            self.input_revision,
        ) else {
            return Ok(Vec::new());
        };
        if reject_unsupported_uri(&mut job, &params.text_document.uri) {
            self.documents.reject_project_input(&job);
            return Ok(vec![job]);
        }
        if self.workspace_input_status == WorkspaceInputStatus::Rebuilding {
            self.documents.mark_project_input_pending(&job);
            return Ok(Vec::new());
        }
        let affected = match self.workspace.upsert_open(
            params.text_document.uri.clone(),
            i64::from(params.text_document.version),
            source,
        ) {
            Ok(affected) => affected,
            Err(error) => {
                self.prepare_project_job(&mut job, Err(error));
                return Ok(vec![job]);
            }
        };
        let project_context = self
            .workspace
            .project_analysis_context(&params.text_document.uri);
        let options = self.analysis_options_for(project_context.as_ref().ok());
        self.documents.update_job_options(&mut job, options);
        self.prepare_project_job(&mut job, project_context);
        let mut jobs = vec![job];
        self.append_dependent_jobs(&affected, params.text_document.uri.as_str(), &mut jobs);
        Ok(jobs)
    }

    fn append_dependent_jobs(
        &mut self,
        affected: &std::collections::BTreeSet<String>,
        changed: &str,
        jobs: &mut Vec<AnalysisJob>,
    ) {
        for uri in affected.iter().filter(|uri| uri.as_str() != changed) {
            let Ok(parsed) = uri.parse() else {
                continue;
            };
            let Some(mut job) = self.documents.begin_reanalysis(uri, self.input_revision) else {
                continue;
            };
            let project_context = self.workspace.project_analysis_context(&parsed);
            self.prepare_project_job(&mut job, project_context);
            jobs.push(job);
        }
    }

    fn analysis_options_for(
        &self,
        workspace: Option<&crate::workspace::ProjectAnalysisContext>,
    ) -> adocweave::AnalysisOptions {
        let mut options = workspace.map_or_else(adocweave::AnalysisOptions::default, |input| {
            input.project_config.analysis.clone()
        });
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

    fn clear_workspace_watch_errors(&mut self) -> bool {
        let changed = self.workspace_watch_error_epoch == self.workspace_input_epoch
            && (!self.workspace_watch_errors.is_empty()
                || self.workspace_watch_errors_overflowed
                || self.workspace_watch_recovery_required);
        self.workspace_watch_errors.clear();
        self.workspace_watch_error_bytes = 0;
        self.workspace_watch_errors_overflowed = false;
        self.workspace_watch_recovery_required = false;
        self.workspace_watch_error_epoch = self.workspace_input_epoch;
        changed
    }

    fn ensure_current_workspace_watch_errors(&mut self) {
        if self.workspace_watch_error_epoch != self.workspace_input_epoch {
            self.clear_workspace_watch_errors();
        }
    }

    fn current_workspace_watch_recovery_required(&self) -> bool {
        self.workspace_watch_error_epoch == self.workspace_input_epoch
            && self.workspace_watch_recovery_required
    }

    fn clear_workspace_watch_error(&mut self, uri: &lsp::Url) -> bool {
        self.ensure_current_workspace_watch_errors();
        let Some(error) = self.workspace_watch_errors.remove(uri.as_str()) else {
            return false;
        };
        self.workspace_watch_error_bytes = self
            .workspace_watch_error_bytes
            .saturating_sub(uri.as_str().len().saturating_add(error.len()));
        true
    }

    fn record_workspace_watch_error(&mut self, uri: &lsp::Url, error: String) -> bool {
        self.ensure_current_workspace_watch_errors();
        if let Some(previous) = self.workspace_watch_errors.get_mut(uri.as_str()) {
            if *previous == error {
                return false;
            }
            let next_bytes = self
                .workspace_watch_error_bytes
                .saturating_sub(previous.len())
                .saturating_add(error.len());
            if next_bytes > MAX_WORKSPACE_WATCH_ERROR_BYTES {
                self.workspace_watch_errors_overflowed = true;
                return true;
            }
            self.workspace_watch_error_bytes = next_bytes;
            *previous = error;
            return true;
        }
        let additional_bytes = uri.as_str().len().saturating_add(error.len());
        if self.workspace_watch_errors.len() >= MAX_WORKSPACE_WATCH_ERRORS
            || self
                .workspace_watch_error_bytes
                .saturating_add(additional_bytes)
                > MAX_WORKSPACE_WATCH_ERROR_BYTES
        {
            let changed = !self.workspace_watch_errors_overflowed;
            self.workspace_watch_errors_overflowed = true;
            return changed;
        }
        self.workspace_watch_error_bytes = self
            .workspace_watch_error_bytes
            .saturating_add(additional_bytes);
        self.workspace_watch_errors.insert(uri.to_string(), error);
        true
    }

    fn workspace_watch_error_message(&self) -> Option<String> {
        if self.workspace_watch_error_epoch != self.workspace_input_epoch
            || (self.workspace_watch_errors.is_empty() && !self.workspace_watch_errors_overflowed)
        {
            return None;
        }
        let mut messages = self
            .workspace_watch_errors
            .iter()
            .map(|(uri, error)| format!("{uri}: {error}"))
            .collect::<Vec<_>>();
        if self.workspace_watch_errors_overflowed {
            messages.push(format!(
                "additional workspace watch errors exceed the retained limit of {MAX_WORKSPACE_WATCH_ERRORS}"
            ));
        }
        Some(messages.join("; "))
    }

    pub(crate) fn workspace_files_changed_with_journal(
        &mut self,
        params: lsp::DidChangeWatchedFilesParams,
    ) -> WorkspaceFileChanges {
        if params.changes.iter().any(|change| {
            change.uri.path_segments().and_then(Iterator::last) == Some(adocweave_config::FILE_NAME)
        }) {
            // Project files are handled by the backend's asynchronous full
            // scan. Mixing an incremental resource reload with a new resource
            // plan would admit files under two different configurations.
            return WorkspaceFileChanges {
                jobs: Vec::new(),
                journal: Vec::new(),
                replay_complete: false,
                recovery_required: false,
            };
        }
        let mut coalesced = std::collections::BTreeMap::<lsp::Url, lsp::FileChangeType>::new();
        let mut uri_bytes = 0_usize;
        for change in params.changes {
            if !coalesced.contains_key(&change.uri) {
                if coalesced.len() >= MAX_WORKSPACE_WATCH_CHANGES
                    || uri_bytes.saturating_add(change.uri.as_str().len())
                        > MAX_WORKSPACE_WATCH_URI_BYTES
                {
                    self.ensure_current_workspace_watch_errors();
                    self.workspace_watch_recovery_required = true;
                    return WorkspaceFileChanges {
                        jobs: Vec::new(),
                        journal: Vec::new(),
                        replay_complete: false,
                        recovery_required: true,
                    };
                }
                uri_bytes = uri_bytes.saturating_add(change.uri.as_str().len());
            }
            coalesced.insert(change.uri, change.typ);
        }
        let changes = coalesced
            .into_iter()
            .map(|(uri, typ)| lsp::FileEvent { uri, typ });
        let mut affected = std::collections::BTreeSet::new();
        let mut journal = Vec::new();
        let mut watch_errors_changed = false;
        for change in changes {
            let kind = if change.typ == lsp::FileChangeType::DELETED {
                WatchedFileKind::Delete
            } else {
                WatchedFileKind::Upsert
            };
            match self.workspace.apply_watched_file(change.uri.clone(), kind) {
                Ok(update) => {
                    watch_errors_changed |= self.clear_workspace_watch_error(&change.uri);
                    affected.extend(update.affected);
                    if update.journal_relevant {
                        journal.push(change);
                    }
                }
                Err(error) => {
                    watch_errors_changed |=
                        self.record_workspace_watch_error(&change.uri, error.message);
                    if error.journal_relevant {
                        journal.push(change);
                    }
                }
            }
        }
        let mut jobs = Vec::new();
        if !affected.is_empty() || watch_errors_changed {
            self.advance_input_revision();
        }
        self.append_dependent_jobs(&affected, "", &mut jobs);
        if watch_errors_changed {
            let queued = jobs
                .iter()
                .map(|job| job.uri.clone())
                .collect::<std::collections::BTreeSet<_>>();
            for (uri, _, _) in self.documents.open_sources() {
                if queued.contains(&uri) {
                    continue;
                }
                let Ok(parsed) = uri.parse::<lsp::Url>() else {
                    continue;
                };
                if parsed.scheme() != "file" {
                    let Some(mut job) = self.documents.reconfigure(
                        &uri,
                        self.analysis_options_for(None),
                        self.input_revision,
                    ) else {
                        continue;
                    };
                    reject_unsupported_uri(&mut job, &parsed);
                    self.documents.reject_project_input(&job);
                    jobs.push(job);
                    continue;
                }
                let project_context = self.workspace.project_analysis_context(&parsed);
                let options = self.analysis_options_for(project_context.as_ref().ok());
                if let Some(mut job) =
                    self.documents
                        .reconfigure(&uri, options, self.input_revision)
                {
                    self.prepare_project_job(&mut job, project_context);
                    jobs.push(job);
                }
            }
        }
        WorkspaceFileChanges {
            jobs,
            journal,
            replay_complete: true,
            recovery_required: self.workspace_watch_error_epoch == self.workspace_input_epoch
                && (self.workspace_watch_errors_overflowed
                    || self.workspace_watch_recovery_required),
        }
    }

    pub(crate) fn handle_workspace_files_changed(
        &mut self,
        params: lsp::DidChangeWatchedFilesParams,
    ) -> WorkspaceFileEventOutcome {
        if params.changes.iter().any(|change| {
            change.uri.path_segments().and_then(Iterator::last) == Some(adocweave_config::FILE_NAME)
        }) {
            self.advance_workspace_input_epoch();
            return WorkspaceFileEventOutcome {
                jobs: self.reconfigure_open_workspace(),
                recovery_generation: None,
                rebuild: None,
                cancel_recovery_timer: false,
            };
        }

        if self.workspace_input_status == WorkspaceInputStatus::Rebuilding {
            let mut uris = std::collections::BTreeSet::new();
            let mut uri_bytes = 0_usize;
            let mut replay_complete = true;
            for change in &params.changes {
                if uris.insert(change.uri.as_str()) {
                    uri_bytes = uri_bytes
                        .checked_add(change.uri.as_str().len())
                        .expect("workspace watch URI byte count exhausted");
                    if uris.len() > MAX_WORKSPACE_WATCH_CHANGES
                        || uri_bytes > MAX_WORKSPACE_WATCH_URI_BYTES
                    {
                        replay_complete = false;
                        break;
                    }
                }
            }
            let changes = WorkspaceFileChanges {
                jobs: Vec::new(),
                journal: if replay_complete {
                    params.changes
                } else {
                    Vec::new()
                },
                replay_complete,
                recovery_required: !replay_complete
                    || self.current_workspace_watch_recovery_required(),
            };
            if !replay_complete {
                self.begin_workspace_rebuild();
            }
            let recovery_generation = self.record_workspace_changes(&changes);
            return WorkspaceFileEventOutcome {
                jobs: Vec::new(),
                recovery_generation,
                rebuild: None,
                cancel_recovery_timer: false,
            };
        }

        let changes = self.workspace_files_changed_with_journal(params);
        let mut jobs = changes.jobs;
        if changes.recovery_required || !changes.replay_complete {
            jobs.extend(self.reconfigure_open_workspace());
        }
        WorkspaceFileEventOutcome {
            jobs,
            recovery_generation: None,
            rebuild: None,
            cancel_recovery_timer: false,
        }
    }

    /// Reads the workspace roots without touching service state.
    ///
    /// This is the half of a reload whose cost grows with the workspace: it
    /// walks every directory below each root and reads the `.adoc` files it
    /// finds. It takes `&self` so a caller can run it on a worker while the
    /// event loop keeps answering requests. Installing the result is
    /// [`Self::apply_workspace_scan`], which is cheap and runs in order.
    #[cfg(test)]
    pub fn plan_workspace_scan(&self, cancellation: &dyn CancellationCheck) -> WorkspaceScan {
        let roots = self.workspace_roots.values().cloned().collect::<Vec<_>>();
        WorkspaceScan {
            loaded: self
                .workspace
                .load_roots_detached_with_cancellation(&roots, cancellation),
        }
    }

    pub(crate) fn plan_workspace_scan_with_job(
        &self,
        cancellation: &dyn CancellationCheck,
        job: &adocweave_host::FilesystemJobCoordinator,
    ) -> WorkspaceScan {
        let roots = self.workspace_roots.values().cloned().collect::<Vec<_>>();
        WorkspaceScan {
            loaded: self
                .workspace
                .load_roots_detached_with_job(&roots, cancellation, job),
        }
    }

    pub(crate) fn record_workspace_changes(
        &mut self,
        changes: &WorkspaceFileChanges,
    ) -> Option<u64> {
        self.workspace_scans
            .lock()
            .expect("workspace scan state lock is poisoned")
            .record_workspace_changes(changes)
    }

    #[allow(dead_code)]
    pub(crate) fn workspace_scan_recovery_is_current(&self, generation: u64) -> bool {
        self.workspace_scans
            .lock()
            .expect("workspace scan state lock is poisoned")
            .debouncing_generation()
            == Some(generation)
    }

    #[allow(dead_code)]
    pub(crate) fn request_workspace_scan_recovery(
        &mut self,
        recovery: WorkspaceScanRecovery,
    ) -> Option<WorkspaceScanStart> {
        self.workspace_scans
            .lock()
            .expect("workspace scan state lock is poisoned")
            .request_recovery(recovery.generation())
    }

    #[allow(dead_code)]
    pub(crate) fn complete_workspace_scan(
        &mut self,
        scanned: WorkspaceScanned,
    ) -> Option<WorkspaceScanTransition> {
        let scan_state = Arc::clone(&self.workspace_scans);
        let mut coordinator = {
            let mut state = scan_state
                .lock()
                .expect("workspace scan state lock is poisoned");
            std::mem::take(&mut *state)
        };
        let transition = coordinator.complete(self, scanned);
        *scan_state
            .lock()
            .expect("workspace scan state lock is poisoned") = coordinator;
        transition
    }

    pub(crate) fn cancel_workspace_scan(&mut self) {
        self.workspace_scans
            .lock()
            .expect("workspace scan state lock is poisoned")
            .cancel();
    }

    /// Installs a completed scan and returns the analyses it makes stale.
    ///
    /// The documents open at this moment are overlaid onto the read, not the
    /// ones open when it started, so a document opened during the walk is kept.
    #[allow(dead_code)]
    pub(crate) fn apply_workspace_scan(&mut self, scan: WorkspaceScan) -> WorkspaceScanApplication {
        let structural_rebuild = self.workspace_input_status == WorkspaceInputStatus::Rebuilding;
        let open_sources = self.documents.open_sources();
        let parsed_open_sources = parse_open_sources(&open_sources);
        let outcome = self
            .workspace
            .apply_loaded_roots(scan.loaded, &parsed_open_sources);
        let installed = outcome.is_ok();
        let structural_rebuild_pending = structural_rebuild && !installed;
        let jobs = if structural_rebuild_pending {
            self.workspace_scan_failed(outcome.expect_err("failed structural workspace install"))
        } else {
            if installed {
                self.documents.synchronize_all_project_inputs();
            }
            self.workspace_input_status = WorkspaceInputStatus::Ready;
            self.advance_input_revision();
            self.finish_reload(outcome, open_sources)
        };
        let notices = if installed {
            let current = self.workspace.scan_notices().clone();
            let newly_active = current
                .difference(&self.active_scan_notices)
                .cloned()
                .collect();
            self.active_scan_notices = current;
            newly_active
        } else {
            Vec::new()
        };
        WorkspaceScanApplication {
            jobs,
            installed,
            structural_rebuild_pending,
            notices,
        }
    }

    /// Records an internal scan worker failure without replacing the last
    /// coherent workspace snapshot.
    #[allow(dead_code)]
    pub fn workspace_scan_failed(&mut self, error: String) -> Vec<AnalysisJob> {
        if self.workspace_input_status == WorkspaceInputStatus::Rebuilding {
            self.set_workspace_error(error);
            self.ensure_current_workspace_watch_errors();
            self.workspace_watch_recovery_required = true;
            return Vec::new();
        }
        self.advance_input_revision();
        self.set_workspace_error(error);
        self.ensure_current_workspace_watch_errors();
        self.workspace_watch_recovery_required = true;
        self.documents
            .open_sources()
            .into_iter()
            .filter_map(|(uri, _, _)| {
                let parsed: lsp::Url = uri.parse().ok()?;
                if parsed.scheme() != "file" {
                    let mut job = self.documents.reconfigure(
                        &uri,
                        self.analysis_options_for(None),
                        self.input_revision,
                    )?;
                    reject_unsupported_uri(&mut job, &parsed);
                    self.documents.reject_project_input(&job);
                    return Some(job);
                }
                let project_context = self.workspace.project_analysis_context(&parsed);
                let options = self.analysis_options_for(project_context.as_ref().ok());
                let mut job = self
                    .documents
                    .reconfigure(&uri, options, self.input_revision)?;
                self.prepare_project_job(&mut job, project_context);
                Some(job)
            })
            .collect()
    }

    /// Turns the result of a reload into the reanalyses it requires.
    fn finish_reload(
        &mut self,
        outcome: Result<(), String>,
        open_sources: Vec<(String, i32, String)>,
    ) -> Vec<AnalysisJob> {
        if let Err(error) = outcome {
            let failed_closed = self.workspace.last_load_failed_closed();
            self.set_workspace_error(error.clone());
            self.ensure_current_workspace_watch_errors();
            self.workspace_watch_recovery_required = true;
            return open_sources
                .into_iter()
                .filter_map(|(uri, _, _)| {
                    let parsed: lsp::Url = uri.parse().ok()?;
                    if parsed.scheme() != "file" {
                        let mut job = self.documents.reconfigure(
                            &uri,
                            self.analysis_options_for(None),
                            self.input_revision,
                        )?;
                        reject_unsupported_uri(&mut job, &parsed);
                        self.documents.reject_project_input(&job);
                        return Some(job);
                    }
                    let project_context = if failed_closed {
                        Err(error.clone())
                    } else {
                        self.workspace.project_analysis_context(&parsed)
                    };
                    let options = self.analysis_options_for(project_context.as_ref().ok());
                    let mut job = self
                        .documents
                        .reconfigure(&uri, options, self.input_revision)?;
                    self.prepare_project_job(&mut job, project_context);
                    Some(job)
                })
                .collect();
        }
        self.workspace_error = None;
        self.clear_workspace_watch_errors();
        open_sources
            .into_iter()
            .filter_map(|(uri, _, _)| {
                let parsed: lsp::Url = uri.parse().ok()?;
                if parsed.scheme() != "file" {
                    let mut job = self.documents.reconfigure(
                        &uri,
                        self.analysis_options_for(None),
                        self.input_revision,
                    )?;
                    reject_unsupported_uri(&mut job, &parsed);
                    self.documents.reject_project_input(&job);
                    return Some(job);
                }
                let project_context = self.workspace.project_analysis_context(&parsed);
                let options = self.analysis_options_for(project_context.as_ref().ok());
                let mut job = self
                    .documents
                    .reconfigure(&uri, options, self.input_revision)?;
                self.prepare_project_job(&mut job, project_context);
                Some(job)
            })
            .collect()
    }

    pub fn workspace_folders_changed(
        &mut self,
        params: lsp::DidChangeWorkspaceFoldersParams,
    ) -> Vec<AnalysisJob> {
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
        self.active_scan_notices.clear();
        self.advance_workspace_input_epoch();
        self.reconfigure_open_workspace()
    }

    fn reconfigure_open_workspace(&mut self) -> Vec<AnalysisJob> {
        self.advance_input_revision();
        self.configure_open_workspace()
    }

    /// Rebuilds authority and project input from the documents open now.
    ///
    /// With no workspace folders, each open file contributes only its parent
    /// directory. Replacing the set on every open and close also drops the
    /// authority for a directory after its last document closes.
    fn configure_open_workspace(&mut self) -> Vec<AnalysisJob> {
        let open_sources = self.documents.open_sources();
        let parsed_open_sources = parse_open_sources(&open_sources);
        let roots = self.workspace_roots.values().cloned().collect::<Vec<_>>();
        let outcome = self.workspace.configure_roots(&roots, &parsed_open_sources);
        if outcome.is_ok() {
            self.documents.synchronize_all_project_inputs();
        }
        self.workspace_input_status = WorkspaceInputStatus::Ready;
        self.finish_reload(outcome, open_sources)
    }

    /// Returns the effective text a workspace resource holds, if it is known.
    #[cfg(test)]
    pub fn workspace_resource(&self, uri: &lsp::Url) -> Option<std::sync::Arc<str>> {
        self.workspace.resource_text(uri)
    }

    /// Returns how many documents the workspace currently holds.
    #[cfg(test)]
    pub fn workspace_analysis_count(&self) -> usize {
        self.workspace.resource_count()
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

    #[cfg(test)]
    pub fn adopt(&mut self, job: &AnalysisJob, result: adocweave::AnalysisResult) -> Adoption {
        if !self.analysis_job_is_current(job) {
            return Adoption::Stale;
        }
        if job
            .project_context
            .as_ref()
            .is_some_and(|input| !self.workspace.project_analysis_context_is_current(input))
        {
            return Adoption::Stale;
        }
        let format = job
            .project_context
            .as_ref()
            .map_or_else(formatter::FormatConfig::default, |input| {
                input.project_config.format
            });
        self.documents.adopt_with_format(job, result, format)
    }

    pub fn adopt_project_result(
        &mut self,
        job: &AnalysisJob,
        mut result: ProjectResult,
        mut sources: ProjectSourceIndex,
    ) -> Vec<String> {
        if !self.analysis_job_is_current(job) {
            return Vec::new();
        }
        if job
            .project_context
            .as_ref()
            .is_some_and(|input| !self.workspace.project_analysis_context_is_current(input))
        {
            return Vec::new();
        }
        self.record_project_result_dependencies(job, &result, &sources);
        let Some(target) = result.targets.pop() else {
            return Vec::new();
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
            let version = existing
                .as_ref()
                .and_then(|value| value.version)
                .or_else(|| sources.version_for_uri(&uri));
            if let Some(version) = version {
                resource_versions.insert(uri.clone(), version);
            }
            sources.insert(
                resource.source_id.clone(),
                ProjectSourceState { uri, text, version },
            );
        }
        let sources = Arc::new(sources);
        let analysis = match target.analysis {
            Ok(analysis) => analysis,
            Err(error) => {
                let _ = self.adopt_project_problem(job, target_problem(job, error));
                return Vec::new();
            }
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
                    job,
                    sources.as_ref(),
                    &target.resources,
                    error,
                )),
            ),
        };
        let mut current_diagnostic_uris = std::collections::BTreeSet::from([job.uri.clone()]);
        current_diagnostic_uris.extend(target.resources.iter().filter_map(|resource| {
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
            current_diagnostic_uris.insert(uri.clone());
        }
        let mut diagnostic_uris_to_refresh = job.previously_published_diagnostic_uris.clone();
        diagnostic_uris_to_refresh.extend(current_diagnostic_uris.iter().cloned());
        if self.documents.adopt_project(
            job,
            ProjectAdoption {
                primary: analysis.primary,
                expanded,
                format,
                sources,
                problem,
                published_diagnostic_uris: current_diagnostic_uris,
            },
        ) == Adoption::Adopted
        {
            diagnostic_uris_to_refresh.into_iter().collect()
        } else {
            Vec::new()
        }
    }

    pub fn record_project_result_dependencies(
        &mut self,
        job: &AnalysisJob,
        result: &ProjectResult,
        sources: &ProjectSourceIndex,
    ) -> bool {
        if !self.analysis_job_is_current(job)
            || job
                .project_context
                .as_ref()
                .is_none_or(|context| !self.workspace.project_analysis_context_is_current(context))
        {
            return false;
        }
        let Some(target) = result.targets.first() else {
            return false;
        };
        let Ok(root_uri) = lsp::Url::parse(&job.uri) else {
            return false;
        };
        let dependency_uri = |resource: &ProjectResourceResult| {
            sources
                .get(&resource.source_id)
                .and_then(|source| lsp::Url::parse(&source.uri).ok())
                .or_else(|| lsp::Url::from_file_path(&resource.path).ok())
        };
        let includes = target.resources.iter().filter_map(|resource| {
            if resource.kind != ProjectResourceKind::Include
                || !(matches!(
                    resource.outcome,
                    ProjectResourceOutcome::Loaded { .. }
                        | ProjectResourceOutcome::LoadedOmitted { .. }
                        | ProjectResourceOutcome::Missing
                ) || matches!(resource.outcome, ProjectResourceOutcome::Failed(_))
                    && resource.observation.is_some())
            {
                return None;
            }
            dependency_uri(resource)
        });
        let local_targets = target.resources.iter().filter_map(|resource| {
            if resource.kind != ProjectResourceKind::LocalTarget
                || !(matches!(
                    resource.outcome,
                    ProjectResourceOutcome::Present | ProjectResourceOutcome::Missing
                ) || matches!(resource.outcome, ProjectResourceOutcome::Failed(_))
                    && resource.observation.is_some())
            {
                return None;
            }
            dependency_uri(resource)
        });
        let _ = self
            .workspace
            .record_project_dependencies(&root_uri, includes, local_targets);
        true
    }

    #[cfg(test)]
    pub(crate) fn workspace_copy(&self) -> crate::workspace::WorkspaceResources {
        self.workspace.clone()
    }

    /// Rebuilds a current job when its captured project context became stale
    /// while the worker was running.
    pub fn refresh_stale_project(&mut self, job: &AnalysisJob) -> Option<AnalysisJob> {
        if !self.analysis_job_is_current(job) {
            return None;
        }
        let context = job.project_context.as_ref()?;
        if self.workspace.project_analysis_context_is_current(context) {
            return None;
        }
        self.rebuild_current_project_job(job)
    }

    pub fn retry_project_analysis(&mut self, job: &AnalysisJob) -> Option<AnalysisJob> {
        if !self.analysis_job_is_current(job) {
            return None;
        }
        self.rebuild_current_project_job(job)
    }

    fn rebuild_current_project_job(&mut self, job: &AnalysisJob) -> Option<AnalysisJob> {
        let uri: lsp::Url = job.uri.parse().ok()?;
        let context = self.workspace.project_analysis_context(&uri);
        let options = self.analysis_options_for(context.as_ref().ok());
        let mut retry = self
            .documents
            .reconfigure(&job.uri, options, self.input_revision)?;
        self.prepare_project_job(&mut retry, context);
        Some(retry)
    }

    pub fn adopt_project_problem(
        &mut self,
        job: &AnalysisJob,
        problem: ProjectProblem,
    ) -> Adoption {
        if !self.analysis_job_is_current(job) {
            return Adoption::Stale;
        }
        if job.project_problem.is_none()
            && job
                .project_context
                .as_ref()
                .is_none_or(|input| !self.workspace.project_analysis_context_is_current(input))
        {
            return Adoption::Stale;
        }
        self.documents.adopt_project_problem(job, problem)
    }

    pub fn close(&mut self, uri: &lsp::Url) -> SessionCloseOutcome {
        let diagnostic_uris = self.documents.published_diagnostic_uris(uri.as_str());
        let closed = self.documents.close(uri.as_str());
        if self.workspace_input_status == WorkspaceInputStatus::Rebuilding {
            if closed {
                self.advance_input_revision();
            }
            return SessionCloseOutcome {
                closed,
                reanalysis_jobs: Vec::new(),
                diagnostic_uris,
            };
        }
        if closed && self.workspace_roots.is_empty() {
            self.advance_input_revision();
            return SessionCloseOutcome {
                closed,
                reanalysis_jobs: self.configure_open_workspace(),
                diagnostic_uris,
            };
        }
        let mut affected = self.workspace.close_open(uri).unwrap_or_else(|error| {
            self.set_workspace_error(error);
            std::collections::BTreeSet::new()
        });
        match self.workspace.forget_project_dependencies(uri) {
            Ok(pruned) => affected.extend(pruned),
            Err(error) => self.set_workspace_error(error),
        }
        if closed {
            self.advance_input_revision();
        }
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
        self.cancel_workspace_scan();
    }

    pub fn document_cancellation(
        &self,
        uri: &lsp::Url,
    ) -> Option<Arc<adocweave::CancellationToken>> {
        self.documents.cancellation(uri.as_str())
    }

    #[cfg(test)]
    pub(crate) fn begin_reanalysis_for_test(&mut self, uri: &lsp::Url) -> Option<AnalysisJob> {
        let mut job = self
            .documents
            .begin_reanalysis(uri.as_str(), self.input_revision)?;
        if reject_unsupported_uri(&mut job, uri) {
            self.documents.reject_project_input(&job);
            return Some(job);
        }
        let project_context = self.workspace.project_analysis_context(uri);
        self.prepare_project_job(&mut job, project_context);
        Some(job)
    }

    pub fn update_configuration(
        &mut self,
        settings: serde_json::Value,
    ) -> Result<Vec<AnalysisJob>, String> {
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
        if self.workspace_input_status == WorkspaceInputStatus::Rebuilding {
            self.invalidate_all_document_inputs();
            return Ok(Vec::new());
        }
        self.advance_input_revision();
        Ok(self
            .documents
            .open_sources()
            .into_iter()
            .filter_map(|(uri, _, _)| {
                let parsed: lsp::Url = uri.parse().ok()?;
                if parsed.scheme() != "file" {
                    let mut job = self.documents.reconfigure(
                        &uri,
                        self.analysis_options_for(None),
                        self.input_revision,
                    )?;
                    reject_unsupported_uri(&mut job, &parsed);
                    self.documents.reject_project_input(&job);
                    return Some(job);
                }
                let project_context = self.workspace.project_analysis_context(&parsed);
                let options = self.analysis_options_for(project_context.as_ref().ok());
                let mut job = self
                    .documents
                    .reconfigure(&uri, options, self.input_revision)?;
                self.prepare_project_job(&mut job, project_context);
                Some(job)
            })
            .collect())
    }

    pub const fn debounce_ms(&self) -> u64 {
        self.settings.debounce_ms
    }

    pub fn diagnostics(&self, uri: &lsp::Url) -> Result<lsp::PublishDiagnosticsParams, String> {
        let workspace_watch_error = self.workspace_watch_error_message();
        let document = self.documents.get(uri.as_str());
        let resource = self.workspace.get(uri);
        let source = document
            .map(|document| document.document_input.source.as_ref())
            .or_else(|| self.documents.adopted_source(uri.as_str()))
            .or_else(|| resource.map(|resource| resource.text().as_ref()));
        let Some(source) = source else {
            return Ok(lsp::PublishDiagnosticsParams::new(
                uri.clone(),
                self.current_workspace_error()
                    .or(workspace_watch_error.as_deref())
                    .map(crate::diagnostics::workspace_error)
                    .into_iter()
                    .collect(),
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
            .and_then(|document| {
                if document.project_input_problem().is_some() {
                    None
                } else {
                    document.view.as_ref().map(|view| view.primary.as_ref())
                }
            })
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
        if let Some(error) = self.current_workspace_error() {
            diagnostics.push(crate::diagnostics::workspace_error(error));
        }
        if let Some(error) = &workspace_watch_error {
            diagnostics.push(crate::diagnostics::workspace_error(error));
        }
        if let Some(problem) = document.and_then(|document| document.project_input_problem()) {
            diagnostics.push(crate::diagnostics::project_problem(
                problem.range,
                &problem.code,
                &problem.message,
                &source_document,
                self.position_encoding,
            )?);
        }
        for expanded in self.documents.expanded_analyses() {
            let current_version = expanded.resource_versions.get(uri.as_str()).copied();
            let is_root = expanded.analysis.source_id().is_some_and(|source_id| {
                expanded.uri_for_source_id(source_id) == Some(uri.as_str())
            });
            if current_version
                != document
                    .map(|document| document.document_input.revision.version)
                    .or_else(|| resource.map(|resource| resource.revision().get()))
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
            navigation::Definition::Host(target) => {
                let request =
                    host_reference_request(&document, uri, target, self.position_encoding);
                cancellation.check_now()?;
                let result = self
                    .host_index
                    .definition(&request)
                    .map(|location| location.map(lsp::GotoDefinitionResponse::Scalar));
                cancellation.check_now()?;
                Ok(result?)
            }
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
        if let Some(target) = result.host_target {
            let request = host_reference_request(&document, uri, target, self.position_encoding);
            cancellation.check_now()?;
            let locations = self.host_index.references(&request, include_declaration);
            cancellation.check_now()?;
            let locations = locations?;
            if let Some(locations) = locations {
                return Ok(Some(locations));
            }
        }
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
        let host_request =
            host_reference_request(document, uri, target.key.clone(), self.position_encoding);
        cancellation.check_now()?;
        let host_locations = self.host_index.references(&host_request, false);
        cancellation.check_now()?;
        let host_locations = host_locations?;
        let mut locations = if let Some(locations) = host_locations {
            locations
        } else {
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
            references.fallback
        };
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
        let mut links = navigation::document_links(&input, uri, tooltips, cancellation)?;
        for unresolved in std::mem::take(&mut links.unresolved) {
            cancellation.checkpoint()?;
            let request = host_reference_request(
                &document,
                uri,
                unresolved.target.clone(),
                self.position_encoding,
            );
            cancellation.check_now()?;
            let location = self.host_index.definition(&request).ok().flatten();
            cancellation.check_now()?;
            links.resolve(unresolved, location, tooltips);
        }
        Ok(Some(links.finish(cancellation)?))
    }

    fn source_document(&self, uri: &lsp::Url) -> Result<SourceDocument, String> {
        let source = self
            .documents
            .get(uri.as_str())
            .map(|document| document.document_input.source.as_ref())
            .or_else(|| self.documents.adopted_source(uri.as_str()))
            .or_else(|| {
                self.workspace
                    .get(uri)
                    .map(|resource| resource.text().as_ref())
            })
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

fn host_reference_request(
    document: &DocumentSnapshot,
    uri: &lsp::Url,
    target: ReferenceKey,
    encoding: PositionEncoding,
) -> HostReferenceRequest {
    HostReferenceRequest {
        source: uri.clone(),
        source_version: revision_version_i32(&document.revision),
        source_generation: document.revision.generation,
        target,
        encoding,
    }
}

fn revision_version_i32(revision: &adocweave::DocumentRevision) -> i32 {
    i32::try_from(revision.version).expect("LSP document versions originate as i32")
}
