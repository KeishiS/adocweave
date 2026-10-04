//! Source selection and protocol preparation, without I/O.

use super::protocol::{
    Citation, CitationItem, Csl, Eqnums, Equation, Macro, Request, Scope, ScopeInput, Scopes,
    Severity,
};
use super::{HostDiagnostic, HostError, HostResult, SourceMap};
use adocweave_core::{
    Analysis,
    output::projection::FormulaKind,
    semantic::{Block, Inline, MathLanguage, SemanticNode, walk},
    text::TextRange,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub equations: Vec<TextRange>,
    pub citations: Vec<TextRange>,
    /// Actually visible footnote occurrence macros, including definitions.
    pub footnotes: Vec<TextRange>,
}
#[derive(Clone, Debug)]
pub struct Prepared {
    pub request: Request,
    pub sources: SourceMap,
    pub diagnostics: Vec<HostDiagnostic>,
}

/// Notes are forcibly excluded before requests, diagnostics and CSL items are collected.
pub fn prepare(
    analysis: &Analysis,
    body: &Selection,
    notes: &Selection,
    include_notes: bool,
    macros: Option<Vec<Macro>>,
    mut csl: Option<Csl>,
) -> HostResult<Prepared> {
    let mut sources = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut inputs = Scopes {
        body: ScopeInput::default(),
        notes: ScopeInput::default(),
    };
    let selected = super::selected_content(analysis, body, notes, include_notes)?;
    let mut stem_ranges = BTreeSet::new();
    walk(analysis.document(), |node| match node {
        SemanticNode::Inline(Inline::Formula(formula)) if formula.uses_stem_attribute => {
            stem_ranges.insert(formula.range);
        }
        SemanticNode::Block(Block::Math(block))
            if block
                .metadata
                .attributes
                .iter()
                .any(|a| a.name.is_none() && a.value == "stem") =>
        {
            stem_ranges.insert(block.range);
        }
        _ => {}
    });
    for (scope, content) in [
        (Scope::Body, &selected.body),
        (Scope::Notes, &selected.notes),
    ] {
        let target = match scope {
            Scope::Body => &mut inputs.body,
            Scope::Notes => &mut inputs.notes,
        };
        stem_ranges.extend(&content.stem_ranges);
        for formula in &content.equations {
            if formula.language != MathLanguage::Latex {
                let mut message = format!(
                    "slides support LaTeX equations; `{}` is unsupported",
                    formula.language.as_asciidoc_name()
                );
                if stem_ranges.contains(&formula.source_range)
                    && let Some(value) = analysis
                        .attribute_environment()
                        .resolve_at("stem", formula.source_range.start())
                    && let Ok(Some(value)) = value.value
                    && !matches!(value, "" | "asciimath" | "latexmath" | "latex" | "tex")
                {
                    message.push_str(&format!(" (stem={value})"));
                }
                diagnostics.push(HostDiagnostic {
                    scope: Some(scope),
                    key: None,
                    range: Some(formula.source_range),
                    severity: Severity::Error,
                    code: "slides-math-unsupported".into(),
                    message,
                });
                continue;
            }
            let key = format!("m{}", target.equations.len());
            sources.insert((scope, key.clone()), formula.source_range);
            target.equations.push(Equation {
                key,
                tex: formula.source.clone(),
                display: formula.kind == FormulaKind::Block,
            });
        }
        for citation in &content.citations {
            // Without external CSL the caller has already checked visible manual definitions.
            if csl.is_none() {
                continue;
            }
            let key = format!("c{}", target.citations.len());
            let mut items = Vec::new();
            for cited in &citation.keys {
                let mut item = CitationItem::new(cited.value.clone());
                for attr in &citation.attributes {
                    match attr.name.as_deref() {
                        Some("locator") => item.locator = Some(attr.value.clone()),
                        Some("label") => item.label = Some(attr.value.clone()),
                        Some("prefix") => item.prefix = Some(attr.value.clone()),
                        Some("suffix") => item.suffix = Some(attr.value.clone()),
                        Some("suppress-author") => {
                            item.suppress_author = Some(
                                boolean(&attr.value)
                                    .map_err(|error| error.at(Some(attr.value_range)))?,
                            )
                        }
                        Some("author-only") => {
                            item.author_only = Some(
                                boolean(&attr.value)
                                    .map_err(|error| error.at(Some(attr.value_range)))?,
                            )
                        }
                        _ => {
                            return Err(HostError::protocol("unsupported citation qualifier")
                                .at(Some(attr.value_range)));
                        }
                    }
                }
                items.push(item);
            }
            sources.insert((scope, key.clone()), citation.range);
            target.citations.push(Citation { key, items });
        }
    }
    // The helper receives only the library items cited by the included scopes.
    if let Some(csl) = &mut csl {
        let ids: BTreeSet<_> = inputs
            .iter()
            .iter()
            .flat_map(|(_, input)| input.citations.iter())
            .flat_map(|c| c.items.iter().map(|i| i.id.as_str()))
            .collect();
        csl.items.retain(|item| {
            item.get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| ids.contains(id))
        });
    }
    let header_end = analysis.document().header().end;
    let eqnums_range = analysis
        .attribute_environment()
        .resolve_at("eqnums", header_end)
        .and_then(|attribute| {
            attribute
                .binding
                .map(|binding| binding.occurrence().value.source_range)
        });
    let eqnums = match analysis
        .attribute_environment()
        .resolve_at("eqnums", header_end)
        .map(|a| a.value)
        .transpose()
        .map_err(|_| {
            HostError::new("slides-eqnums-invalid", "eqnums attribute expansion failed")
                .at(eqnums_range)
        })?
        .flatten()
    {
        None | Some("none") => Eqnums::None,
        Some("" | "AMS") => Eqnums::Ams,
        Some("all") => Eqnums::All,
        Some(_) => {
            return Err(HostError::new(
                "slides-eqnums-invalid",
                "eqnums must be empty, AMS, all, or none",
            )
            .at(eqnums_range));
        }
    };
    let request = Request {
        schema_version: 1,
        eqnums,
        scopes: inputs,
        macros,
        csl,
    };
    request.validate()?;
    Ok(Prepared {
        request,
        sources,
        diagnostics,
    })
}
fn boolean(value: &str) -> HostResult<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(HostError::protocol(
            "citation author flags must be true or false",
        )),
    }
}
