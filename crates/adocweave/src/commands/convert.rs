use std::collections::BTreeMap;
use std::fmt::Write as _;

use adocweave_core::output::diagnostics::Diagnostic;
use adocweave_core::output::html::{HtmlOutput, RenderPolicy};
use adocweave_core::preprocess::{ExpandedRange, OriginRange, PreprocessedAnalysis, SourceOrigin};
use adocweave_core::text::{PositionEncoding, SourceDocument, TextSize};

use crate::check_output::{DiagnosticCounts, ProjectSourceView};

use super::html_policy;

#[derive(Debug)]
pub(crate) enum Error {
    Html(html_policy::Error),
    Position(adocweave_core::text::PositionError),
    Serialize(String),
}

pub(crate) struct DiagnosticReport {
    pub(crate) output: String,
    pub(crate) counts: DiagnosticCounts,
}

pub(crate) fn render_analysis(
    analysis: &adocweave_core::Analysis,
    render_policy: &RenderPolicy,
) -> Result<HtmlOutput, Error> {
    html_policy::render_checked(analysis.document(), render_policy).map_err(Error::Html)
}

pub(crate) fn render_diagnostics(
    analysis: &PreprocessedAnalysis,
    diagnostics: &[Diagnostic],
    sources: &BTreeMap<adocweave_core::SourceId, ProjectSourceView<'_>>,
) -> Result<DiagnosticReport, Error> {
    let mut output = String::new();
    let mut counts = DiagnosticCounts::default();
    for diagnostic in diagnostics {
        let mut origins = analysis
            .document
            .origins_for_range(ExpandedRange::new(diagnostic.range));
        // An empty document has no source-map segments, but diagnostics at
        // its start still belong to the primary source.
        if origins.is_empty()
            && analysis.document.source.is_empty()
            && diagnostic.range.is_empty()
            && diagnostic.range.start() == TextSize::ZERO
        {
            origins.push(SourceOrigin {
                source_id: analysis.analysis.source_id().cloned(),
                range: OriginRange::new(diagnostic.range),
            });
        }
        if origins.is_empty() {
            return Err(Error::Serialize(
                "render diagnostic has no original source range".to_owned(),
            ));
        }
        for origin in origins {
            let source_id = origin
                .source_id
                .as_ref()
                .or_else(|| analysis.analysis.source_id());
            let view = source_id
                .and_then(|source_id| sources.get(source_id))
                .ok_or_else(|| {
                    Error::Serialize(format!(
                        "project result has no source body for {}",
                        source_id.map_or("<unknown>", adocweave_core::SourceId::as_str)
                    ))
                })?;
            let source = SourceDocument::new(view.source).map_err(Error::Position)?;
            let position = source
                .offset_to_position(origin.range.start(), PositionEncoding::Utf8)
                .map_err(Error::Position)?;
            counts.add(diagnostic.severity);
            writeln!(
                output,
                "{}:{}:{}: {}[{}]: {}",
                view.display_id,
                position.line + 1,
                position.character + 1,
                diagnostic.severity.as_str(),
                diagnostic.code.as_str(),
                diagnostic.message,
            )
            .expect("writing to a String cannot fail");
        }
    }
    Ok(DiagnosticReport { output, counts })
}

#[cfg(test)]
mod tests {
    use adocweave_core::output::diagnostics::Severity;
    use adocweave_core::preprocess::{
        EffectiveProcessingOptions, PreprocessInputs, PreprocessOptions, ResourceSnapshot,
    };
    use adocweave_core::resolution::{
        GeneratedBibliography, GeneratedBibliographyEntry, RenderInputs,
    };
    use adocweave_core::{AnalysisOptions, SourceId};

    use crate::check_output::FailOn;

    use super::*;

    fn analyze(source: &str) -> PreprocessedAnalysis {
        EffectiveProcessingOptions::new(
            AnalysisOptions::default(),
            PreprocessOptions {
                source_id: Some(SourceId::new("document")),
                ..PreprocessOptions::default()
            },
        )
        .expect("options")
        .preprocess_and_analyze(
            source,
            &ResourceSnapshot::default(),
            PreprocessInputs::default(),
        )
        .expect("analysis")
    }

    fn sources(source: &str) -> BTreeMap<SourceId, ProjectSourceView<'_>> {
        BTreeMap::from([(
            SourceId::new("document"),
            ProjectSourceView {
                display_id: "document.adoc".to_owned(),
                source,
            },
        )])
    }

    #[test]
    fn conversion_retains_render_diagnostics_without_changing_safe_html() {
        let source = "link:javascript:alert(1)[unsafe]\n";
        let analysis = analyze(source);
        let output = render_analysis(&analysis.analysis, &RenderPolicy::default())
            .expect("rendered document");

        assert_eq!(output.html, "<p>unsafe</p>\n");
        assert_eq!(output.diagnostics.len(), 1);
        assert_eq!(output.diagnostics[0].code.as_str(), "invalid-url-scheme");
        let report = render_diagnostics(&analysis, &output.diagnostics, &sources(source))
            .expect("diagnostics");
        assert!(
            report
                .output
                .starts_with("document.adoc:1:6: warning[invalid-url-scheme]:")
        );
        assert!(!report.counts.fails(FailOn::Error));
    }

    #[test]
    fn only_error_render_diagnostics_reach_the_failure_threshold() {
        let source = "link:javascript:alert(1)[unsafe]\n";
        let analysis = analyze(source);
        let mut output = render_analysis(&analysis.analysis, &RenderPolicy::default())
            .expect("rendered document");

        for severity in [
            Severity::Error,
            Severity::Warning,
            Severity::Information,
            Severity::Hint,
        ] {
            output.diagnostics[0].severity = severity;
            let report = render_diagnostics(&analysis, &output.diagnostics, &sources(source))
                .expect("diagnostics");
            assert_eq!(
                report.counts.fails(FailOn::Error),
                severity == Severity::Error
            );
            assert!(report.output.contains(severity.as_str()));
        }
    }

    #[test]
    fn empty_document_render_errors_still_have_a_primary_source_location() {
        let analysis = analyze("");
        let inputs =
            RenderInputs::default().with_generated_bibliography(GeneratedBibliography::new(
                "References",
                vec![
                    GeneratedBibliographyEntry::new("one", "First.").with_number(1),
                    GeneratedBibliographyEntry::new("two", "Second.").with_number(1),
                ],
            ));
        let output = adocweave_core::output::html::render_with_inputs(
            analysis.analysis.document(),
            &RenderPolicy::default(),
            &inputs,
        );
        let report = render_diagnostics(&analysis, &output.diagnostics, &sources(""))
            .expect("diagnostics at an empty document start");

        assert!(report.counts.fails(FailOn::Error));
        assert!(
            report
                .output
                .contains("document.adoc:1:1: error[invalid-generated-bibliography-numbering]:")
        );
    }

    #[test]
    fn an_unmapped_render_range_is_rejected_instead_of_using_expanded_offsets() {
        let source = "link:javascript:alert(1)[unsafe]\n";
        let analysis = analyze(source);
        let mut output = render_analysis(&analysis.analysis, &RenderPolicy::default())
            .expect("rendered document");
        let outside = TextSize::new(source.len() + 1).expect("offset");
        output.diagnostics[0].range =
            adocweave_core::text::TextRange::new(outside, outside).expect("empty range");

        assert!(matches!(
            render_diagnostics(&analysis, &output.diagnostics, &sources(source)),
            Err(Error::Serialize(message)) if message == "render diagnostic has no original source range"
        ));
    }
}
