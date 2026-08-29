use std::collections::BTreeMap;

use adocweave::output::diagnostics as diagnostic;
use adocweave::text::{PositionEncoding, SourceDocument};

use crate::check_output::{
    CheckOutcome, DiagnosticCounts, DiagnosticFormat, FailOn, github_annotation, sarif_log,
    sarif_result,
};
use crate::{diagnostic_json, local_target};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Options {
    pub(crate) format: DiagnosticFormat,
    pub(crate) fail_on: FailOn,
    pub(crate) summary: bool,
    pub(crate) fix: bool,
    pub(crate) diff: bool,
    pub(crate) enabled_rules: Vec<diagnostic::LintRuleId>,
}

#[derive(Debug)]
pub(crate) enum Error {
    Position(adocweave::text::PositionError),
    Serialize(String),
}

pub(crate) struct ProjectSourceView<'source> {
    pub(crate) display_id: String,
    pub(crate) source: &'source str,
}

pub(crate) fn process_project(
    analysis: &adocweave_project::ProjectAnalysis,
    check: &Options,
    sources: &BTreeMap<adocweave::SourceId, ProjectSourceView<'_>>,
) -> Result<CheckOutcome, Error> {
    let projected = &analysis.source_mapping;
    let source_view = |source_id: Option<&adocweave::SourceId>| {
        let source_id = source_id.or_else(|| analysis.preprocessed.analysis.source_id());
        source_id
            .and_then(|source_id| sources.get(source_id))
            .ok_or_else(|| {
                Error::Serialize(format!(
                    "project result has no source body for {}",
                    source_id.map_or("<unknown>", adocweave::SourceId::as_str)
                ))
            })
    };
    let mut host = analysis
        .local_target_diagnostics
        .iter()
        .map(|item| {
            let view = source_view(Some(&item.source_id))?;
            Ok(local_target::diagnostic_from_project(
                &item.diagnostic,
                &view.display_id,
                view.source,
                &item.target,
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    host.sort_by(|left, right| {
        (
            left.source_id.as_str(),
            left.range.start(),
            left.range.end(),
            left.code.as_str(),
            left.target.as_str(),
        )
            .cmp(&(
                right.source_id.as_str(),
                right.range.start(),
                right.range.end(),
                right.code.as_str(),
                right.target.as_str(),
            ))
    });
    let mut counts = DiagnosticCounts::default();
    for item in &projected.diagnostics {
        for _ in &item.origins {
            counts.add(item.diagnostic.severity);
        }
    }
    counts.add_host_errors(host.len());

    if check.format == DiagnosticFormat::Json {
        let mut values = Vec::new();
        for diagnostic in &projected.diagnostics {
            for origin in &diagnostic.origins {
                let view = source_view(origin.source_id.as_ref())?;
                values.push(diagnostic_json::projected_record_with_source(
                    diagnostic,
                    origin,
                    Some(&view.display_id),
                ));
            }
        }
        values.extend(local_target::json_values(&host));
        return serde_json::to_string(&values)
            .map(|output| CheckOutcome { output, counts })
            .map_err(|error| Error::Serialize(error.to_string()));
    }

    if check.format == DiagnosticFormat::Sarif {
        let mut results = Vec::new();
        for diagnostic in &projected.diagnostics {
            for origin in &diagnostic.origins {
                let view = source_view(origin.source_id.as_ref())?;
                let index = SourceDocument::new(view.source).map_err(Error::Position)?;
                let position = index
                    .offset_to_position(origin.range.start(), PositionEncoding::Utf8)
                    .map_err(Error::Position)?;
                let id = if view.display_id == "<stdin>" {
                    diagnostic.diagnostic.id.as_str().to_owned()
                } else {
                    format!(
                        "{}@{}:{}:{}",
                        diagnostic.diagnostic.code.as_str(),
                        view.display_id,
                        origin.range.start().to_u32(),
                        origin.range.end().to_u32()
                    )
                };
                results.push(sarif_result(
                    &id,
                    diagnostic.diagnostic.severity,
                    diagnostic.diagnostic.code.as_str(),
                    &diagnostic.diagnostic.message,
                    &view.display_id,
                    position.line + 1,
                    position.character + 1,
                ));
            }
        }
        results.extend(host.iter().map(|item| {
            let id = format!(
                "{}@{}:{}:{}",
                item.code,
                item.source_id,
                item.range.start().to_u32(),
                item.range.end().to_u32()
            );
            sarif_result(
                &id,
                diagnostic::Severity::Error,
                &item.code,
                &item.message,
                &item.source_id,
                item.line,
                item.column,
            )
        }));
        return Ok(CheckOutcome {
            output: sarif_log(results),
            counts,
        });
    }

    let mut output = String::new();
    for diagnostic in &projected.diagnostics {
        for origin in &diagnostic.origins {
            let view = source_view(origin.source_id.as_ref())?;
            let index = SourceDocument::new(view.source).map_err(Error::Position)?;
            let position = index
                .offset_to_position(origin.range.start(), PositionEncoding::Utf8)
                .map_err(Error::Position)?;
            if check.format == DiagnosticFormat::Github {
                output.push_str(&github_annotation(
                    diagnostic.diagnostic.severity,
                    diagnostic.diagnostic.code.as_str(),
                    &diagnostic.diagnostic.message,
                    &view.display_id,
                    position.line + 1,
                    position.character + 1,
                ));
            } else {
                use std::fmt::Write as _;
                writeln!(
                    output,
                    "{}:{}:{}: {}[{}]: {}",
                    view.display_id,
                    position.line + 1,
                    position.character + 1,
                    diagnostic.diagnostic.severity.as_str(),
                    diagnostic.diagnostic.code.as_str(),
                    diagnostic.diagnostic.message,
                )
                .expect("writing to a String cannot fail");
            }
        }
    }
    for item in &host {
        if check.format == DiagnosticFormat::Github {
            output.push_str(&github_annotation(
                diagnostic::Severity::Error,
                &item.code,
                &item.message,
                &item.source_id,
                item.line,
                item.column,
            ));
            continue;
        }
        let view = sources
            .values()
            .find(|view| view.display_id == item.source_id)
            .ok_or_else(|| Error::Serialize("project local-target source is missing".to_owned()))?;
        output.push_str(
            &local_target::render_human(std::slice::from_ref(item), view.source)
                .map_err(Error::Position)?,
        );
    }
    Ok(CheckOutcome { output, counts })
}
