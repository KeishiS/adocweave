//! Command-line diagnostic serialization and presentation.

use std::fmt::Write as _;

use adocweave::output::diagnostics::{Diagnostic, LINT_RULES, sort_diagnostics};
use adocweave::text::{PositionEncoding, PositionError, SourceDocument};

pub(crate) fn render_human(
    diagnostics: &[Diagnostic],
    source_document: &SourceDocument,
    encoding: PositionEncoding,
) -> Result<String, PositionError> {
    let mut diagnostics = diagnostics.to_vec();
    sort_diagnostics(&mut diagnostics);
    let mut output = String::new();

    for diagnostic in diagnostics {
        let start = source_document.offset_to_position(diagnostic.range.start(), encoding)?;
        writeln!(
            output,
            "{}:{}: {}[{}]: {}",
            start.line + 1,
            start.character + 1,
            diagnostic.severity.as_str(),
            diagnostic.code.as_str(),
            diagnostic.message
        )
        .expect("writing to a String cannot fail");
    }

    Ok(output)
}

#[cfg(test)]
pub(crate) fn render_json(diagnostics: &[Diagnostic]) -> String {
    let mut diagnostics = diagnostics.to_vec();
    sort_diagnostics(&mut diagnostics);
    serde_json::to_string(&diagnostics).expect("diagnostics are serializable")
}

pub(crate) fn render_lint_rule_catalog_json() -> String {
    let mut rules = LINT_RULES.iter().collect::<Vec<_>>();
    rules.sort_by_key(|descriptor| descriptor.id.as_str());
    serde_json::to_string(&serde_json::json!({
        "schemaVersion": 1,
        "packageVersion": adocweave::VERSION,
        "rules": rules
            .into_iter()
            .map(|descriptor| serde_json::json!({
                "code": descriptor.id.as_str(),
                "defaultSeverity": descriptor.default_severity.as_str(),
                "enabledByDefault": descriptor.default_enabled,
                "description": descriptor.description,
                "fixable": descriptor.fixable,
                "userConfigurable": descriptor.user_configurable,
            }))
            .collect::<Vec<_>>(),
    }))
    .expect("lint rule catalog contains only serializable values")
}

#[cfg(test)]
mod tests {
    use adocweave::output::diagnostics::{
        Applicability, DiagnosticCode, DiagnosticId, Fix, RelatedInformation, Severity, TextEdit,
        lint_rule,
    };
    use adocweave::text::{TextRange, TextSize};

    use super::*;

    fn diagnostic(id: &str, code: &str, start: usize) -> Diagnostic {
        let start = TextSize::new(start).expect("small test offset");
        let end = TextSize::new(start.to_usize() + 1).expect("small test offset");
        Diagnostic {
            id: DiagnosticId::new(id),
            code: DiagnosticCode::new(code),
            severity: Severity::Warning,
            range: TextRange::new(start, end).expect("ordered range"),
            message: format!("message for {id}"),
            related: Vec::new(),
            fixes: Vec::new(),
        }
    }

    #[test]
    fn human_and_json_outputs_use_canonical_order() {
        let diagnostics = [diagnostic("later", "z", 4), diagnostic("first", "a", 0)];
        let document = SourceDocument::new("a\nb\nc\n").expect("source");

        assert_eq!(
            render_human(&diagnostics, &document, PositionEncoding::Utf8),
            Ok(
                "1:1: warning[a]: message for first\n3:1: warning[z]: message for later\n"
                    .to_owned()
            )
        );
        let value: serde_json::Value =
            serde_json::from_str(&render_json(&diagnostics)).expect("diagnostics JSON");
        assert_eq!(value[0]["id"], "first");
        assert_eq!(value[1]["id"], "later");
    }

    #[test]
    fn human_output_uses_one_based_utf16_positions() {
        let document = SourceDocument::new("日本語\nproblem\n").expect("source");
        let mut diagnostic = diagnostic("parse-1", "parse-error", 10);
        diagnostic.severity = Severity::Error;
        diagnostic.range = TextRange::new(
            TextSize::new(10).expect("small test offset"),
            TextSize::new(17).expect("small test offset"),
        )
        .expect("ordered range");
        diagnostic.message = "問題です".to_owned();

        assert_eq!(
            render_human(&[diagnostic], &document, PositionEncoding::Utf16),
            Ok("2:1: error[parse-error]: 問題です\n".to_owned())
        );
    }

    #[test]
    fn json_output_escapes_nested_diagnostic_data() {
        let mut diagnostic = diagnostic("quoted", "quoted-message", 3);
        diagnostic.message = "quote: \" and newline\n".to_owned();
        diagnostic.related = vec![RelatedInformation {
            range: TextRange::new(
                TextSize::new(0).expect("small test offset"),
                TextSize::new(1).expect("small test offset"),
            )
            .expect("ordered range"),
            message: "関連".to_owned(),
        }];
        diagnostic.fixes = vec![
            Fix::new(
                "replace",
                Applicability::Always,
                vec![TextEdit {
                    range: diagnostic.range,
                    replacement: "\"".to_owned(),
                }],
            )
            .expect("valid fix"),
        ];

        let json = render_json(&[diagnostic]);
        assert!(json.contains("quote: \\\" and newline\\n"));
        assert!(json.contains("\"message\":\"関連\""));
        assert!(json.contains("\"replacement\":\"\\\"\""));
        serde_json::from_str::<serde_json::Value>(&json).expect("valid JSON");
    }

    #[test]
    fn lint_catalog_is_sorted_and_matches_the_public_catalog() {
        let value: serde_json::Value =
            serde_json::from_str(&render_lint_rule_catalog_json()).expect("lint catalog JSON");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["packageVersion"], adocweave::VERSION);
        let codes = value["rules"]
            .as_array()
            .expect("rules")
            .iter()
            .map(|rule| rule["code"].as_str().expect("code"))
            .collect::<Vec<_>>();
        assert!(codes.is_sorted());
        assert!(codes.iter().all(|code| lint_rule(code).is_some()));
        assert_eq!(codes.len(), LINT_RULES.len());
    }
}
