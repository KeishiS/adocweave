use adocweave::output::diagnostics as diagnostic;
use adocweave::output::html::HtmlDocumentMode;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) output: String,
}

pub(crate) fn run_project(snapshot: &adocweave_project::ProjectConfigSnapshot) -> Outcome {
    let config = &snapshot.config;
    let analysis = config.analysis();
    let attributes = analysis
        .attributes
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                serde_json::json!({ "state": if value.is_some() { "set" } else { "unset" } }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let rules = diagnostic::LINT_RULES
        .iter()
        .map(|descriptor| {
            let settings = analysis.diagnostics.lint.rule(descriptor.id);
            (
                descriptor.id.as_str().to_owned(),
                serde_json::json!({
                    "enabled": settings.enabled,
                    "severity": settings.severity.as_str(),
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let path = |path: &std::path::Path| path.to_string_lossy().into_owned();
    let limits = config.resource_limits();
    let format = config.format();
    Outcome {
        output: serde_json::to_string_pretty(&serde_json::json!({
            "schemaVersion": config.schema_version(),
            "source": snapshot.path.as_deref().map(path),
            "analysis": {
                "syntaxMode": match analysis.syntax.syntax_mode {
                    adocweave::SyntaxMode::Permissive => "permissive",
                    adocweave::SyntaxMode::Strict => "strict",
                },
                "attributes": attributes,
            },
            "lint": {
                "rules": rules,
                "maxLineLength": analysis.diagnostics.lint.max_line_length,
                "maxConsecutiveBlankLines": analysis.diagnostics.lint.max_consecutive_blank_lines,
                "maxDiagnostics": analysis.diagnostics.lint.max_diagnostics,
            },
            "resources": {
                "include": config.include_enabled(),
                "roots": config.resource_roots().iter().map(|value| path(value)).collect::<Vec<_>>(),
                "maxFiles": limits.max_files,
                "maxTotalBytes": limits.max_total_bytes,
                "maxResourceBytes": limits.max_resource_bytes,
            },
            "workspace": {
                "scan": {
                    "exclude": config.workspace_excludes().collect::<Vec<_>>(),
                },
            },
            "localTargets": {
                "enabled": config.local_targets_enabled(),
                "projectRoot": config.local_target_root().map(path),
            },
            "format": {
                "newline": match format.newline {
                    adocweave::output::formatter::NewlineStyle::Lf => "lf",
                    adocweave::output::formatter::NewlineStyle::CrLf => "cr-lf",
                },
                "finalNewline": format.final_newline,
                "maxConsecutiveBlankLines": format.max_consecutive_blank_lines,
            },
            "html": {
                "complete": config.html_policy().document_mode == HtmlDocumentMode::Complete,
                "stylesheetFiles": config.stylesheet_files().iter().map(|value| path(value)).collect::<Vec<_>>(),
                "stylesheetUrls": config.stylesheet_urls(),
            }
        }))
        .expect("resolved configuration is serializable"),
    }
}
