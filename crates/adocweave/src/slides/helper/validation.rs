//! Request-relative validation of every result before rendering.

use super::protocol::{self, CitationResult, EquationResult, Response, Scope, Scopes, Severity};
use super::{HostDiagnostic, HostError, HostResult, Prepared};
use adocweave_core::{
    output::html::{
        BibliographyNamespace, ResolvedMath, ResolvedRichCitation, ValidatedMath, ValidatedRichText,
    },
    resolution::{GeneratedBibliography, GeneratedBibliographyEntry, RenderInputs},
};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct ValidatedResults {
    pub inputs: Scopes<RenderInputs>,
    pub diagnostics: Vec<HostDiagnostic>,
    pub notices: protocol::Notices,
}

/// IDs provided here include authored IDs and generated slide/container IDs.
pub fn validate_response(
    prepared: &Prepared,
    response: Response,
    exit_code: i32,
    reserved_ids: &BTreeSet<String>,
) -> HostResult<ValidatedResults> {
    prepared.request.validate()?;
    protocol::check(
        response.schema_version == 1,
        "unsupported response schemaVersion",
    )?;
    protocol::check(
        response.diagnostics.len() <= protocol::ITEMS * 8,
        "helper diagnostic count limit exceeded",
    )?;
    let has_errors = response
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error);
    protocol::check(
        exit_code == if has_errors { 1 } else { 0 },
        "helper exit status does not match its diagnostics",
    )?;
    let mut diagnostics = prepared.diagnostics.clone();
    for d in &response.diagnostics {
        protocol::check(
            !d.code.is_empty()
                && d.code.len() <= 128
                && d.code
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "invalid helper diagnostic code",
        )?;
        protocol::check(
            d.message.len() <= 16 * 1024,
            "helper diagnostic byte limit exceeded",
        )?;
        protocol::check(
            d.key.is_none() || d.scope.is_some(),
            "helper diagnostic key requires a scope",
        )?;
        let range = match (d.scope, d.key.as_ref()) {
            (Some(scope), Some(key)) => {
                let input = prepared.request.scopes.get(scope);
                protocol::check(
                    input.equations.iter().any(|e| &e.key == key)
                        || input.citations.iter().any(|c| &c.key == key),
                    "diagnostic names an unknown request key",
                )?;
                Some(
                    *prepared
                        .sources
                        .get(&(scope, key.clone()))
                        .ok_or_else(|| HostError::protocol("diagnostic names an unknown key"))?,
                )
            }
            _ => None,
        };
        if d.scope == Some(Scope::Notes) {
            protocol::check(
                !prepared.request.scopes.notes.equations.is_empty()
                    || !prepared.request.scopes.notes.citations.is_empty(),
                "unexpected notes diagnostic",
            )?;
        }
        diagnostics.push(HostDiagnostic {
            scope: d.scope,
            key: d.key.clone(),
            range,
            severity: d.severity,
            code: d.code.clone(),
            message: d.message.clone(),
        });
    }
    let fatal = exit_code == 1
        && response.scopes.iter().iter().all(|(_, scope)| {
            scope.equations.is_empty()
                && scope.citations.is_empty()
                && scope.bibliography.is_empty()
        })
        && response
            .diagnostics
            .iter()
            .any(|d| d.scope.is_none() && d.key.is_none() && d.severity == Severity::Error);
    response.notices.validate(&prepared.request, fatal)?;
    if fatal {
        return Err(HostError::new(
            "slides-helper-failed",
            response
                .diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Error)
                .map(|d| format!("{}: {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    let mut inputs = Scopes {
        body: RenderInputs::default(),
        notes: RenderInputs::default(),
    };
    let mut all_ids = reserved_ids.clone();
    let mut math_refs = Vec::new();
    let known_items: BTreeSet<_> = prepared
        .request
        .csl
        .as_ref()
        .into_iter()
        .flat_map(|c| c.items.iter())
        .filter_map(|i| i.get("id").and_then(serde_json::Value::as_str))
        .collect();
    for (scope, output) in response.scopes.iter() {
        let request = prepared.request.scopes.get(scope);
        protocol::check(
            output.equations.len() == request.equations.len()
                && output.citations.len() == request.citations.len(),
            "helper result count mismatch",
        )?;
        let mut keys = BTreeSet::new();
        let mut math = Vec::new();
        let mut citations = Vec::new();
        for result in &output.equations {
            protocol::check(
                request.equations.iter().any(|r| r.key == result.key())
                    && keys.insert(result.key()),
                "unknown or duplicate equation result key",
            )?;
            validate_status(scope, result.key(), result.failed(), &response)?;
            if let EquationResult::Ok { key, svg, mathml } = result {
                let value =
                    ValidatedMath::validate(scope.as_str(), key, svg, mathml).map_err(|e| {
                        HostError::protocol(e.to_string())
                            .at(prepared.sources.get(&(scope, key.clone())).copied())
                    })?;
                for id in value.ids() {
                    protocol::check(
                        all_ids.insert(id.clone()),
                        "math ID collides with another generated or authored ID",
                    )?;
                }
                math_refs.extend(value.references().iter().cloned());
                math.push(ResolvedMath::new(source(prepared, scope, key)?, value));
            }
        }
        for result in &output.citations {
            protocol::check(
                request.citations.iter().any(|r| r.key == result.key())
                    && keys.insert(result.key()),
                "unknown or duplicate citation result key",
            )?;
            validate_status(scope, result.key(), result.failed(), &response)?;
            if let CitationResult::Ok { key, inlines } = result {
                let value = validate_rich(inlines.clone()).map_err(|e| {
                    HostError::protocol(e.to_string())
                        .at(prepared.sources.get(&(scope, key.clone())).copied())
                })?;
                citations.push(ResolvedRichCitation::new(
                    source(prepared, scope, key)?,
                    value,
                ));
            }
        }
        protocol::check(
            output.bibliography.len() <= protocol::BIBLIOGRAPHY_ITEMS,
            "bibliography result count limit exceeded",
        )?;
        let cited: BTreeSet<_> = request
            .citations
            .iter()
            .flat_map(|c| c.items.iter().map(|i| i.id.as_str()))
            .collect();
        let mut bibliography_ids = BTreeSet::new();
        let mut entries = Vec::new();
        for entry in &output.bibliography {
            protocol::check(
                known_items.contains(entry.id.as_str())
                    && cited.contains(entry.id.as_str())
                    && bibliography_ids.insert(&entry.id),
                "unknown, uncited or duplicate bibliography result id",
            )?;
            let value = validate_rich(entry.inlines.clone())
                .map_err(|e| HostError::protocol(e.to_string()))?;
            entries.push(
                GeneratedBibliographyEntry::new(&entry.id, value.plain_text())
                    .with_rich_text(value),
            );
        }
        let mut rendered = RenderInputs::default()
            .with_math(math)
            .with_rich_citations(citations);
        if !entries.is_empty() {
            rendered = rendered.with_generated_bibliography(
                GeneratedBibliography::new("References", entries).with_namespace(match scope {
                    Scope::Body => BibliographyNamespace::SlidesBody,
                    Scope::Notes => BibliographyNamespace::SlidesNotes,
                }),
            );
        }
        match scope {
            Scope::Body => inputs.body = rendered,
            Scope::Notes => inputs.notes = rendered,
        }
    }
    for target in math_refs {
        protocol::check(
            all_ids.contains(&target) && !reserved_ids.contains(&target),
            "math fragment target has no validated SVG definition",
        )?;
    }
    Ok(ValidatedResults {
        inputs,
        diagnostics,
        notices: response.notices,
    })
}
fn validate_status(scope: Scope, key: &str, failed: bool, response: &Response) -> HostResult<()> {
    let has_error = response.diagnostics.iter().any(|d| {
        d.scope == Some(scope) && d.key.as_deref() == Some(key) && d.severity == Severity::Error
    });
    protocol::check(
        failed == has_error,
        "failed result and scoped error diagnostic disagree",
    )
}
fn source(
    prepared: &Prepared,
    scope: Scope,
    key: &str,
) -> HostResult<adocweave_core::text::TextRange> {
    prepared
        .sources
        .get(&(scope, key.to_owned()))
        .copied()
        .ok_or_else(|| HostError::protocol("result has no Rust source range"))
}

fn validate_rich(
    inlines: Vec<adocweave_core::output::html::RichInline>,
) -> Result<ValidatedRichText, String> {
    let value = ValidatedRichText::validate(inlines).map_err(|e| e.to_string())?;
    for href in value.links() {
        let url = url::Url::parse(href).map_err(|_| "invalid citation HTTP(S) URL".to_owned())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(
                "citation link must be an absolute HTTP(S) URL without credentials".to_owned(),
            );
        }
    }
    Ok(value)
}
