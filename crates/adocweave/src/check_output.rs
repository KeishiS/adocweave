//! Stable diagnostic output and CI failure contracts.

use adocweave_core::output::diagnostics as diagnostic;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum DiagnosticFormat {
    #[default]
    Human,
    Json,
    Github,
    Sarif,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum FailOn {
    #[default]
    Error,
    Warning,
    Never,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DiagnosticCounts {
    errors: usize,
    warnings: usize,
    information: usize,
    hints: usize,
}

impl DiagnosticCounts {
    pub(crate) fn add(&mut self, severity: diagnostic::Severity) {
        match severity {
            diagnostic::Severity::Error => self.errors += 1,
            diagnostic::Severity::Warning => self.warnings += 1,
            diagnostic::Severity::Information => self.information += 1,
            diagnostic::Severity::Hint => self.hints += 1,
        }
    }

    pub(crate) fn add_host_errors(&mut self, count: usize) {
        self.errors += count;
    }

    pub(crate) const fn fails(self, threshold: FailOn) -> bool {
        match threshold {
            FailOn::Error => self.errors > 0,
            FailOn::Warning => self.errors > 0 || self.warnings > 0,
            FailOn::Never => false,
        }
    }

    pub(crate) fn summary(self) -> String {
        format!(
            "errors={}, warnings={}, information={}, hints={}",
            self.errors, self.warnings, self.information, self.hints
        )
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.errors += other.errors;
        self.warnings += other.warnings;
        self.information += other.information;
        self.hints += other.hints;
    }
}

pub(crate) struct CheckOutcome {
    pub(crate) output: String,
    pub(crate) counts: DiagnosticCounts,
}

pub(crate) fn github_annotation(
    severity: diagnostic::Severity,
    code: &str,
    message: &str,
    source_id: &str,
    line: u32,
    column: u32,
) -> String {
    let command = match severity {
        diagnostic::Severity::Error => "error",
        diagnostic::Severity::Warning => "warning",
        diagnostic::Severity::Information | diagnostic::Severity::Hint => "notice",
    };
    format!(
        "::{command} file={},line={line},col={column},title={}::{}\n",
        github_property(source_id),
        github_property(code),
        github_message(message)
    )
}

fn github_message(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn github_property(value: &str) -> String {
    github_message(value)
        .replace(':', "%3A")
        .replace(',', "%2C")
}

pub(crate) fn sarif_result(
    diagnostic_id: &str,
    severity: diagnostic::Severity,
    code: &str,
    message: &str,
    source_id: &str,
    line: u32,
    column: u32,
) -> serde_json::Value {
    let level = match severity {
        diagnostic::Severity::Error => "error",
        diagnostic::Severity::Warning => "warning",
        diagnostic::Severity::Information | diagnostic::Severity::Hint => "note",
    };
    serde_json::json!({
        "ruleId": code,
        "level": level,
        "message": { "text": message },
        "partialFingerprints": {
            "adocweaveDiagnosticId": diagnostic_id,
        },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": { "uri": source_id },
                "region": {
                    "startLine": line,
                    "startColumn": column,
                }
            }
        }]
    })
}

pub(crate) fn sarif_log(results: Vec<serde_json::Value>) -> String {
    serde_json::to_string(&serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "AdocWeave",
                    "version": adocweave_core::VERSION,
                    "informationUri": "https://github.com/KeishiS/adocweave",
                }
            },
            "results": results,
        }]
    }))
    .expect("SARIF diagnostics are serializable")
}

pub(crate) fn sarif_results(output: &str) -> Vec<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(output)
        .ok()
        .and_then(|value| {
            value
                .get("runs")?
                .get(0)?
                .get("results")?
                .as_array()
                .cloned()
        })
        .expect("check SARIF contains one run with a results array")
}
