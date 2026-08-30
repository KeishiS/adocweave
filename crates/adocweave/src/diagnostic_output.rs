//! Shared diagnostic serialization for CLI metadata and preview tests.

use adocweave_core::output::diagnostics::LINT_RULES;
#[cfg(test)]
use adocweave_core::output::diagnostics::{Diagnostic, sort_diagnostics};

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
        "packageVersion": adocweave_core::VERSION,
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
    use adocweave_core::output::diagnostics::{DiagnosticCode, DiagnosticId, Severity, lint_rule};
    use adocweave_core::text::{TextRange, TextSize};

    use super::*;

    #[test]
    fn lint_catalog_is_sorted_and_matches_the_public_catalog() {
        let value: serde_json::Value =
            serde_json::from_str(&render_lint_rule_catalog_json()).expect("lint catalog JSON");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["packageVersion"], adocweave_core::VERSION);
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

    #[test]
    fn diagnostic_json_uses_canonical_order() {
        let diagnostic = |id: &str, code: &str, start: usize| Diagnostic {
            id: DiagnosticId::new(id),
            code: DiagnosticCode::new(code),
            severity: Severity::Warning,
            range: TextRange::new(
                TextSize::new(start).expect("small test offset"),
                TextSize::new(start + 1).expect("small test offset"),
            )
            .expect("ordered range"),
            message: id.to_owned(),
            related: Vec::new(),
            fixes: Vec::new(),
        };
        let value: serde_json::Value = serde_json::from_str(&render_json(&[
            diagnostic("later", "z", 4),
            diagnostic("first", "a", 0),
        ]))
        .expect("diagnostics JSON");
        assert_eq!(value[0]["id"], "first");
        assert_eq!(value[1]["id"], "later");
    }
}
