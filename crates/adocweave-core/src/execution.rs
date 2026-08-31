//! Runtime-independent contracts for scheduling and adopting core analysis.

use std::sync::Arc;

use crate::{
    Analysis, AnalysisInputs, AnalysisOptions, CancellationCheck, Engine, ParseError, SourceId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentRevision {
    pub source_id: Option<SourceId>,
    pub version: i64,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisRequest {
    pub revision: DocumentRevision,
    pub source: Arc<str>,
    pub options: AnalysisOptions,
}

impl AnalysisRequest {
    pub fn new(
        source_id: Option<SourceId>,
        version: i64,
        generation: u64,
        source: impl Into<Arc<str>>,
        options: AnalysisOptions,
    ) -> Self {
        Self {
            revision: DocumentRevision {
                source_id,
                version,
                generation,
            },
            source: source.into(),
            options,
        }
    }

    pub fn analyze(
        &self,
        cancellation: &dyn CancellationCheck,
    ) -> Result<AnalysisResult, ParseError> {
        let analysis = Engine::new(self.options.clone()).analyze_with(
            &self.source,
            AnalysisInputs {
                source_id: self.revision.source_id.as_ref(),
                cancellation: Some(cancellation),
            },
        )?;
        Ok(AnalysisResult {
            revision: self.revision.clone(),
            analysis,
        })
    }
}

#[derive(Debug)]
pub struct AnalysisResult {
    pub revision: DocumentRevision,
    pub analysis: Analysis,
}

impl AnalysisResult {
    pub fn is_current(
        &self,
        current: &DocumentRevision,
        cancellation: &dyn CancellationCheck,
    ) -> bool {
        !cancellation.is_cancelled() && self.revision == *current
    }
}

#[cfg(test)]
mod tests {
    use crate::{CancellationToken, NeverCancel};

    use super::*;

    fn request(source: &str) -> AnalysisRequest {
        AnalysisRequest::new(
            Some(SourceId::new("host:one")),
            1,
            1,
            Arc::<str>::from(source),
            AnalysisOptions::default(),
        )
    }

    #[test]
    fn cancellation_and_revision_gate_result_adoption() {
        let request = request("= Current");
        let result = request.analyze(&NeverCancel).expect("analysis");
        assert!(result.is_current(&request.revision, &NeverCancel));

        let stale = DocumentRevision {
            generation: request.revision.generation + 1,
            ..request.revision.clone()
        };
        assert!(!result.is_current(&stale, &NeverCancel));

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(!result.is_current(&request.revision, &cancelled));
    }
}
