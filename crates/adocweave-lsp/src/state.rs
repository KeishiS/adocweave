//! Versioned document state and generation-checked analysis adoption.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[cfg(test)]
use adocweave::AnalysisResult;
use adocweave::preprocess::{AnalysisProjection, PreprocessedDocument};
use adocweave::text::TextRange;
use adocweave::{
    Analysis, AnalysisOptions, CancellationCheck, CancellationToken, DocumentRevision, SourceId,
};
use adocweave_project::{ProjectLocalTargetDiagnostic, ProjectObservationAccess, ProjectRequest};

#[derive(Debug)]
pub struct ProjectAnalysisSnapshot {
    pub uri: String,
    pub document_input: DocumentAnalysisInput,
    pub cancellation: Arc<CancellationToken>,
    pub project_problem: Option<ProjectProblem>,
    pub prepared_request: Option<PreparedProjectRequest>,
    pub previously_published_diagnostic_uris: BTreeSet<String>,
}

#[derive(Debug)]
pub struct PreparedProjectRequest {
    pub request: ProjectRequest,
    pub source_index: ProjectSourceIndex,
    pub observation_access: ProjectObservationAccess,
    #[cfg(test)]
    pub(crate) _synthetic_root: Option<tempfile::TempDir>,
}

#[derive(Clone, Debug)]
pub struct DocumentAnalysisInput {
    pub revision: DocumentRevision,
    pub source: Arc<str>,
    pub options: AnalysisOptions,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectSourceIndex {
    by_id: BTreeMap<SourceId, ProjectSourceState>,
}

#[derive(Clone, Debug)]
pub struct ProjectSourceState {
    pub uri: String,
    pub text: Arc<str>,
    pub version: Option<i64>,
    pub generation: Option<u64>,
}

impl ProjectSourceIndex {
    pub fn insert(&mut self, source_id: SourceId, source: ProjectSourceState) {
        self.by_id.insert(source_id, source);
    }

    pub fn get(&self, source_id: &SourceId) -> Option<&ProjectSourceState> {
        self.by_id.get(source_id)
    }

    pub fn source_for_uri(&self, uri: &str) -> Option<&ProjectSourceState> {
        self.by_id.values().find(|source| source.uri == uri)
    }

    pub fn open_document_revisions(&self) -> impl Iterator<Item = (&str, i64, u64)> {
        self.by_id
            .values()
            .filter_map(|source| Some((source.uri.as_str(), source.version?, source.generation?)))
    }
}

#[derive(Clone, Debug)]
pub struct DocumentState {
    pub source_id: SourceId,
    pub document_input: DocumentAnalysisInput,
    pub view: Option<Arc<DocumentView>>,
    pub project_problem: Option<ProjectProblem>,
    published_diagnostic_uris: BTreeSet<String>,
    cancellation: Arc<CancellationToken>,
}

#[derive(Debug)]
pub struct DocumentView {
    pub primary: Arc<Analysis>,
    pub expanded: Option<Arc<ExpandedDocumentAnalysis>>,
    pub format: adocweave::output::formatter::FormatConfig,
    pub sources: Arc<ProjectSourceIndex>,
}

pub struct AdoptedAnalysis {
    pub primary: Option<Analysis>,
    pub expanded: Option<ExpandedDocumentAnalysis>,
    pub format: adocweave::output::formatter::FormatConfig,
    pub sources: Arc<ProjectSourceIndex>,
    pub problem: Option<ProjectProblem>,
    pub published_diagnostic_uris: BTreeSet<String>,
}

#[cfg(test)]
impl DocumentState {
    pub fn analysis(&self) -> Option<&Analysis> {
        self.view.as_ref().map(|view| view.primary.as_ref())
    }
}

impl DocumentState {
    fn published_diagnostic_uris(&self, document_uri: &str) -> BTreeSet<String> {
        let mut uris = self.published_diagnostic_uris.clone();
        uris.insert(document_uri.to_owned());
        uris
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectProblem {
    pub document_uri: Option<String>,
    pub range: TextRange,
    pub code: String,
    pub message: String,
}

#[derive(Debug)]
pub struct ExpandedDocumentAnalysis {
    pub document: Arc<PreprocessedDocument>,
    pub analysis: Arc<Analysis>,
    pub projection: Arc<AnalysisProjection>,
    pub resource_versions: BTreeMap<String, i64>,
    pub local_target_diagnostics: Vec<ProjectLocalTargetDiagnostic>,
    pub sources: Arc<ProjectSourceIndex>,
}

impl ExpandedDocumentAnalysis {
    pub fn uri_for_source_id(&self, source_id: &SourceId) -> Option<&str> {
        self.sources
            .get(source_id)
            .map(|source| source.uri.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct DocumentSnapshot {
    pub uri: String,
    pub revision: DocumentRevision,
    pub analysis: Arc<Analysis>,
    pub format: adocweave::output::formatter::FormatConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Adoption {
    Adopted,
    Stale,
    Closed,
}

#[derive(Clone, Debug, Default)]
pub struct DocumentStore {
    documents: Arc<BTreeMap<String, DocumentState>>,
    next_generation: u64,
}

impl DocumentStore {
    pub fn get(&self, uri: &str) -> Option<&DocumentState> {
        self.documents.get(uri)
    }

    pub fn snapshot(&self, uri: &str) -> Option<DocumentSnapshot> {
        let document = self.documents.get(uri)?;
        Some(DocumentSnapshot {
            uri: document.source_id.as_str().to_owned(),
            revision: document.document_input.revision.clone(),
            analysis: document.view.as_ref()?.primary.clone(),
            format: document.view.as_ref()?.format,
        })
    }

    pub fn snapshots(&self) -> Vec<DocumentSnapshot> {
        self.documents
            .values()
            .filter_map(|document| self.snapshot(document.source_id.as_str()))
            .collect()
    }

    pub fn open_sources(&self) -> Vec<(String, i32, String)> {
        self.documents
            .values()
            .filter_map(|document| {
                Some((
                    document.source_id.as_str().to_owned(),
                    i32::try_from(document.document_input.revision.version).ok()?,
                    document.document_input.source.to_string(),
                ))
            })
            .collect()
    }

    pub fn open_project_sources(&self) -> Vec<(String, DocumentRevision, String)> {
        self.documents
            .values()
            .map(|document| {
                (
                    document.source_id.as_str().to_owned(),
                    document.document_input.revision.clone(),
                    document.document_input.source.to_string(),
                )
            })
            .collect()
    }

    pub fn revision_is_current(&self, uri: &str, version: i64, generation: u64) -> bool {
        self.documents.get(uri).is_some_and(|document| {
            document.document_input.revision.version == version
                && document.document_input.revision.generation == generation
        })
    }

    pub fn expanded_analyses(&self) -> impl Iterator<Item = &ExpandedDocumentAnalysis> {
        self.documents
            .values()
            .filter_map(|document| document.view.as_ref()?.expanded.as_deref())
    }

    pub fn project_problems(&self) -> impl Iterator<Item = &ProjectProblem> {
        self.documents
            .values()
            .filter_map(|document| document.project_problem.as_ref())
    }

    pub fn adopted_source(&self, uri: &str) -> Option<&str> {
        self.documents.values().find_map(|document| {
            document
                .view
                .as_ref()?
                .sources
                .source_for_uri(uri)
                .map(|source| source.text.as_ref())
        })
    }

    pub fn cancellation(&self, uri: &str) -> Option<Arc<CancellationToken>> {
        self.documents
            .get(uri)
            .map(|document| document.cancellation.clone())
    }

    pub fn published_diagnostic_uris(&self, uri: &str) -> BTreeSet<String> {
        self.documents
            .get(uri)
            .map(|document| document.published_diagnostic_uris(uri))
            .unwrap_or_default()
    }

    pub fn snapshot_is_current(&self, snapshot: &ProjectAnalysisSnapshot) -> bool {
        self.documents.get(&snapshot.uri).is_some_and(|document| {
            document.document_input.revision == snapshot.document_input.revision
                && !snapshot.cancellation.is_cancelled()
        })
    }

    #[cfg(test)]
    pub fn begin_open(
        &mut self,
        uri: String,
        version: i32,
        text: String,
    ) -> ProjectAnalysisSnapshot {
        self.begin_open_with_options(uri, version, text, AnalysisOptions::default())
    }

    pub fn begin_open_with_options(
        &mut self,
        uri: String,
        version: i32,
        text: String,
        options: AnalysisOptions,
    ) -> ProjectAnalysisSnapshot {
        if let Some(previous) = self.documents.get(&uri) {
            previous.cancellation.cancel();
        }
        let source_id = SourceId::new(uri.clone());
        let snapshot = self.new_snapshot(uri.clone(), source_id.clone(), version, text, options);
        Arc::make_mut(&mut self.documents).insert(
            uri.clone(),
            DocumentState {
                source_id,
                document_input: snapshot.document_input.clone(),
                view: None,
                project_problem: None,
                published_diagnostic_uris: BTreeSet::from([uri]),
                cancellation: snapshot.cancellation.clone(),
            },
        );
        snapshot
    }

    pub fn begin_change(
        &mut self,
        uri: &str,
        version: i32,
        text: String,
    ) -> Option<ProjectAnalysisSnapshot> {
        let current = self.documents.get(uri)?;
        if i64::from(version) <= current.document_input.revision.version {
            return None;
        }
        let options = current.document_input.options.clone();
        let source_id = current.source_id.clone();
        current.cancellation.cancel();
        let snapshot = self.new_snapshot(uri.to_owned(), source_id, version, text, options);
        self.install_snapshot(uri, &snapshot);
        Some(snapshot)
    }

    pub fn begin_reanalysis(&mut self, uri: &str) -> Option<ProjectAnalysisSnapshot> {
        let current = self.documents.get(uri)?;
        current.cancellation.cancel();
        let version = i32::try_from(current.document_input.revision.version).ok()?;
        let text = current.document_input.source.to_string();
        let options = current.document_input.options.clone();
        let source_id = current.source_id.clone();
        let snapshot = self.new_snapshot(uri.to_owned(), source_id, version, text, options);
        self.install_snapshot(uri, &snapshot);
        Some(snapshot)
    }

    pub fn reconfigure(
        &mut self,
        uri: &str,
        options: AnalysisOptions,
    ) -> Option<ProjectAnalysisSnapshot> {
        let current = self.documents.get(uri)?;
        current.cancellation.cancel();
        let version = i32::try_from(current.document_input.revision.version).ok()?;
        let text = current.document_input.source.to_string();
        let source_id = current.source_id.clone();
        let snapshot = self.new_snapshot(uri.to_owned(), source_id, version, text, options);
        self.install_snapshot(uri, &snapshot);
        Some(snapshot)
    }

    /// Replaces an existing document's pending request with `snapshot`, clearing the
    /// prior analysis view and project problem.
    fn install_snapshot(&mut self, uri: &str, snapshot: &ProjectAnalysisSnapshot) {
        let current = Arc::make_mut(&mut self.documents)
            .get_mut(uri)
            .expect("document existence checked");
        current.document_input = snapshot.document_input.clone();
        current.view = None;
        current.project_problem = None;
        current.cancellation = snapshot.cancellation.clone();
    }

    #[cfg(test)]
    pub fn adopt(
        &mut self,
        snapshot: &ProjectAnalysisSnapshot,
        result: AnalysisResult,
    ) -> Adoption {
        self.adopt_with_format(
            snapshot,
            result,
            adocweave::output::formatter::FormatConfig::default(),
        )
    }

    #[cfg(test)]
    pub fn adopt_with_format(
        &mut self,
        snapshot: &ProjectAnalysisSnapshot,
        result: AnalysisResult,
        format: adocweave::output::formatter::FormatConfig,
    ) -> Adoption {
        let Some(document) = self.documents.get(&snapshot.uri) else {
            return Adoption::Closed;
        };
        if !result.is_current(
            &document.document_input.revision,
            snapshot.cancellation.as_ref(),
        ) {
            return Adoption::Stale;
        }
        let document = Arc::make_mut(&mut self.documents)
            .get_mut(&snapshot.uri)
            .expect("document existence checked");
        document.view = Some(Arc::new(DocumentView {
            primary: Arc::new(result.analysis),
            expanded: None,
            format,
            sources: Arc::new(ProjectSourceIndex::default()),
        }));
        Adoption::Adopted
    }

    /// Confirms the document for `job` still exists and its revision has not
    /// been superseded or cancelled, returning the terminal [`Adoption`] on
    /// failure.
    fn ensure_current(&self, snapshot: &ProjectAnalysisSnapshot) -> Result<(), Adoption> {
        let Some(document) = self.documents.get(&snapshot.uri) else {
            return Err(Adoption::Closed);
        };
        if document.document_input.revision != snapshot.document_input.revision
            || snapshot.cancellation.is_cancelled()
        {
            return Err(Adoption::Stale);
        }
        Ok(())
    }

    pub fn complete_analysis(
        &mut self,
        snapshot: &ProjectAnalysisSnapshot,
        analysis: AdoptedAnalysis,
    ) -> Adoption {
        if let Err(adoption) = self.ensure_current(snapshot) {
            return adoption;
        }
        let document = Arc::make_mut(&mut self.documents)
            .get_mut(&snapshot.uri)
            .expect("document existence checked");
        document.view = analysis.primary.map(|primary| {
            Arc::new(DocumentView {
                primary: Arc::new(primary),
                expanded: analysis.expanded.map(Arc::new),
                format: analysis.format,
                sources: analysis.sources,
            })
        });
        document.project_problem = analysis.problem;
        document.published_diagnostic_uris = analysis.published_diagnostic_uris;
        Adoption::Adopted
    }

    pub fn close(&mut self, uri: &str) -> bool {
        if !self.documents.contains_key(uri) {
            return false;
        }
        let document = Arc::make_mut(&mut self.documents)
            .remove(uri)
            .expect("document existence checked");
        document.cancellation.cancel();
        true
    }

    pub fn cancel_all(&mut self) {
        for document in self.documents.values() {
            document.cancellation.cancel();
        }
    }

    fn new_snapshot(
        &mut self,
        uri: String,
        source_id: SourceId,
        version: i32,
        text: String,
        options: AnalysisOptions,
    ) -> ProjectAnalysisSnapshot {
        let previously_published_diagnostic_uris = self.published_diagnostic_uris(&uri);
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("document analysis generation exhausted");
        let document_input = DocumentAnalysisInput {
            revision: DocumentRevision {
                source_id: Some(source_id),
                version: i64::from(version),
                generation: self.next_generation,
            },
            source: Arc::from(text),
            options,
        };
        ProjectAnalysisSnapshot {
            uri,
            document_input,
            cancellation: Arc::new(CancellationToken::new()),
            project_problem: None,
            prepared_request: None,
            previously_published_diagnostic_uris,
        }
    }
}

#[cfg(test)]
mod tests {
    use adocweave::{CancellationCheck, NeverCancel};

    use super::{Adoption, DocumentStore, ProjectAnalysisSnapshot};

    fn analyze(snapshot: &ProjectAnalysisSnapshot) -> adocweave::AnalysisResult {
        adocweave::AnalysisRequest {
            revision: snapshot.document_input.revision.clone(),
            source: snapshot.document_input.source.clone(),
            options: snapshot.document_input.options.clone(),
        }
        .analyze(&NeverCancel)
        .expect("analysis")
    }

    #[test]
    fn notification_order_newer_generation_cancels_and_rejects_previous_analysis() {
        let mut store = DocumentStore::default();
        let old = store.begin_open("file:///a.adoc".to_owned(), 1, "= Old".to_owned());
        let new = store
            .begin_change("file:///a.adoc", 2, "= New".to_owned())
            .expect("new generation");

        assert!(old.cancellation.is_cancelled());
        assert!(!new.cancellation.is_cancelled());
        assert_eq!(store.adopt(&old, analyze(&old)), Adoption::Stale);
        assert_eq!(store.adopt(&new, analyze(&new)), Adoption::Adopted);
        let snapshot = store.snapshot("file:///a.adoc").expect("snapshot");
        assert_eq!(snapshot.revision.version, 2);
        assert_eq!(
            snapshot.revision.generation,
            new.document_input.revision.generation
        );
        assert_eq!(snapshot.analysis.source(), "= New");
    }

    #[test]
    fn pending_and_closed_documents_never_expose_an_analysis_snapshot() {
        let mut store = DocumentStore::default();
        let job = store.begin_open("file:///a.adoc".to_owned(), 1, "= A".to_owned());
        assert!(store.snapshot("file:///a.adoc").is_none());

        assert!(store.close("file:///a.adoc"));
        assert!(job.cancellation.is_cancelled());
        assert_eq!(store.adopt(&job, analyze(&job)), Adoption::Closed);
        assert!(store.snapshot("file:///a.adoc").is_none());
    }

    #[test]
    fn shutdown_cancels_every_open_document() {
        let mut store = DocumentStore::default();
        let first = store.begin_open("file:///a.adoc".to_owned(), 1, "= A".to_owned());
        let second = store.begin_open("file:///b.adoc".to_owned(), 1, "= B".to_owned());

        store.cancel_all();

        assert!(first.cancellation.is_cancelled());
        assert!(second.cancellation.is_cancelled());
    }

    #[test]
    fn cloned_store_is_an_owned_copy_on_write_snapshot() {
        let mut store = DocumentStore::default();
        let first = store.begin_open("file:///a.adoc".to_owned(), 1, "= Old".to_owned());
        assert_eq!(store.adopt(&first, analyze(&first)), Adoption::Adopted);
        let snapshot = store.clone();

        let second = store
            .begin_change("file:///a.adoc", 2, "= New".to_owned())
            .expect("new generation");
        assert_eq!(store.adopt(&second, analyze(&second)), Adoption::Adopted);

        assert_eq!(
            snapshot
                .snapshot("file:///a.adoc")
                .expect("old snapshot")
                .analysis
                .source(),
            "= Old"
        );
        assert_eq!(
            store
                .snapshot("file:///a.adoc")
                .expect("new snapshot")
                .analysis
                .source(),
            "= New"
        );
    }

    #[test]
    #[should_panic(expected = "document analysis generation exhausted")]
    fn analysis_generation_is_never_reused_after_exhaustion() {
        let mut store = DocumentStore {
            next_generation: u64::MAX,
            ..DocumentStore::default()
        };
        let _ = store.begin_open("file:///a.adoc".to_owned(), 1, "= A".to_owned());
    }
}
