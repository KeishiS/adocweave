use adocweave::SourceId;

use crate::response_conversion::{wasm_diagnostics, wasm_document_symbols, wasm_text_range};
use crate::{
    AdocWeaveError, AnalyzeResult, AttributeBindingQuery, AttributeExpansionError,
    AttributeQueryResult, AttributeReferenceQuery, AttributeValueContinuation,
    DocumentAttributeContinuation, DocumentAttributeOccurrence, DocumentAttributeOperation,
    DocumentAttributeValue, DocumentAttributeValueLine, MacroForm, ResourcePurpose, ResourceQuery,
    TextRange, serialization_error,
};

pub(crate) struct ResponseProducts {
    pub(crate) syntax: Option<String>,
    pub(crate) canonical_ast: Option<String>,
    pub(crate) html: Option<adocweave::output::html::HtmlOutput>,
    pub(crate) attribute_occurrences: Option<Vec<adocweave::semantic::DocumentAttributeOccurrence>>,
    pub(crate) attribute_queries: Option<adocweave::semantic::AttributeQueryProduct>,
    pub(crate) resource_queries: Option<Vec<adocweave::resolution::ResourceQuery>>,
    pub(crate) diagnostics: Option<Vec<adocweave::output::diagnostics::Diagnostic>>,
    pub(crate) symbols: Option<Vec<adocweave::semantic::DocumentSymbol>>,
    pub(crate) document: Option<crate::DocumentView>,
}

pub(crate) fn project_response(
    products: ResponseProducts,
    source_id: Option<&SourceId>,
    attribute_projection: Option<&adocweave::preprocess::AnalysisProjection>,
) -> Result<AnalyzeResult, AdocWeaveError> {
    let diagnostics = products.diagnostics.map(|mut diagnostics| {
        if let Some(html) = &products.html {
            diagnostics.extend(html.diagnostics.iter().cloned());
        }
        wasm_diagnostics(&diagnostics)
    });
    Ok(AnalyzeResult {
        syntax: products.syntax,
        canonical_ast: products.canonical_ast,
        html: products.html.map(|output| output.html),
        attribute_occurrences: products.attribute_occurrences.map(|occurrences| {
            occurrences
                .iter()
                .map(wasm_document_attribute_occurrence)
                .collect()
        }),
        attribute_queries: products
            .attribute_queries
            .map(|queries| wasm_attribute_query_product(&queries, source_id, attribute_projection)),
        resource_queries: products.resource_queries.map(|queries| {
            queries
                .into_iter()
                .map(|query| {
                    let reference = query.reference;
                    ResourceQuery {
                        purpose: match reference.purpose() {
                            adocweave::resolution::ResourcePurpose::Image => ResourcePurpose::Image,
                            adocweave::resolution::ResourcePurpose::Icon => ResourcePurpose::Icon,
                            adocweave::resolution::ResourcePurpose::Audio => ResourcePurpose::Audio,
                            adocweave::resolution::ResourcePurpose::Video => ResourcePurpose::Video,
                            adocweave::resolution::ResourcePurpose::VideoPoster => {
                                ResourcePurpose::VideoPoster
                            }
                        },
                        form: match reference.form() {
                            adocweave::semantic::MacroForm::Inline => MacroForm::Inline,
                            adocweave::semantic::MacroForm::Block => MacroForm::Block,
                        },
                        owner_range: wasm_text_range(reference.owner_range()),
                        range: wasm_text_range(reference.range()),
                        target_range: wasm_text_range(reference.target_range()),
                        target: reference.target().to_owned(),
                    }
                })
                .collect()
        }),
        diagnostics,
        symbols: products.symbols.map(wasm_document_symbols),
        document: products.document.map(Some),
    })
}

pub(crate) fn enforce_output_limit(
    response: &AnalyzeResult,
    max_output_bytes: usize,
) -> Result<(), AdocWeaveError> {
    let output_bytes = serde_json::to_vec(&response)
        .map_err(serialization_error)?
        .len();
    if output_bytes > max_output_bytes {
        return Err(AdocWeaveError {
            code: "output-limit-exceeded".to_owned(),
            message: format!(
                "output bytes limit exceeded (limit {max_output_bytes}, actual {output_bytes})"
            ),
        });
    }
    Ok(())
}

fn wasm_document_attribute_occurrence(
    occurrence: &adocweave::semantic::DocumentAttributeOccurrence,
) -> DocumentAttributeOccurrence {
    DocumentAttributeOccurrence {
        range: wasm_text_range(occurrence.range),
        name_range: wasm_text_range(occurrence.name_range),
        name: occurrence.name.clone(),
        value: DocumentAttributeValue {
            source_range: wasm_text_range(occurrence.value.source_range),
            source_text: occurrence.value.source_text.clone(),
            folded_text: occurrence.value.folded_text.clone(),
            lines: occurrence
                .value
                .lines
                .iter()
                .map(|line| DocumentAttributeValueLine {
                    range: wasm_text_range(line.range),
                    indent_range: wasm_text_range(line.indent_range),
                    content_range: wasm_text_range(line.content_range),
                    ending_range: wasm_text_range(line.ending_range),
                    continuation: line.continuation.map(|continuation| {
                        DocumentAttributeContinuation {
                            kind: match continuation.kind {
                                adocweave::semantic::AttributeValueContinuation::Soft => {
                                    AttributeValueContinuation::Soft
                                }
                                adocweave::semantic::AttributeValueContinuation::Hard => {
                                    AttributeValueContinuation::Hard
                                }
                            },
                            range: wasm_text_range(continuation.range),
                        }
                    }),
                })
                .collect(),
        },
        operation: match occurrence.operation {
            adocweave::semantic::DocumentAttributeOperation::Set => DocumentAttributeOperation::Set,
            adocweave::semantic::DocumentAttributeOperation::Unset => {
                DocumentAttributeOperation::Unset
            }
            adocweave::semantic::DocumentAttributeOperation::Counter => {
                DocumentAttributeOperation::Counter
            }
        },
        valid: occurrence.valid,
    }
}

fn wasm_attribute_query_product(
    product: &adocweave::semantic::AttributeQueryProduct,
    source_id: Option<&SourceId>,
    projection: Option<&adocweave::preprocess::AnalysisProjection>,
) -> AttributeQueryResult {
    let source_id = source_id.map(|source_id| source_id.as_str().to_owned());
    AttributeQueryResult {
        bindings: product
            .bindings
            .iter()
            .map(|binding| {
                let (effective_value, error) = wasm_attribute_resolution(binding.value());
                let projected = projection.and_then(|projection| {
                    projection
                        .attribute_bindings
                        .iter()
                        .find(|candidate| candidate.value.id() == binding.id())
                });
                let mut occurrence = wasm_document_attribute_occurrence(binding.occurrence());
                if let Some(projected_occurrence) = projection.and_then(|projection| {
                    projection
                        .attribute_occurrences
                        .iter()
                        .find(|candidate| candidate.value.range == binding.occurrence().range)
                }) {
                    wasm_project_document_attribute_occurrence(
                        &mut occurrence,
                        projected_occurrence,
                    );
                }
                let (binding_source_id, range) = wasm_projected_range(
                    projected.map(|value| value.origins.as_slice()),
                    source_id.clone(),
                    binding.occurrence().range,
                );
                occurrence.range = range;
                occurrence.name_range = wasm_projected_range(
                    projected.map(|value| value.name_origins.as_slice()),
                    binding_source_id.clone(),
                    binding.occurrence().name_range,
                )
                .1;
                occurrence.value.source_range = wasm_projected_range(
                    projected.map(|value| value.value_origins.as_slice()),
                    binding_source_id.clone(),
                    binding.occurrence().value.source_range,
                )
                .1;
                AttributeBindingQuery {
                    id: binding.id().get(),
                    source_id: binding_source_id,
                    operation: match binding.operation() {
                        adocweave::semantic::DocumentAttributeOperation::Set => {
                            DocumentAttributeOperation::Set
                        }
                        adocweave::semantic::DocumentAttributeOperation::Unset => {
                            DocumentAttributeOperation::Unset
                        }
                        adocweave::semantic::DocumentAttributeOperation::Counter => {
                            DocumentAttributeOperation::Counter
                        }
                    },
                    effective_value,
                    error,
                    occurrence,
                }
            })
            .collect(),
        references: product
            .references
            .iter()
            .enumerate()
            .map(|(index, reference)| {
                let (effective_value, error) = wasm_attribute_resolution(
                    reference
                        .value
                        .as_ref()
                        .map(|value| value.as_deref())
                        .map_err(|error| *error),
                );
                let projected =
                    projection.and_then(|projection| projection.attribute_references.get(index));
                let (reference_source_id, range) = wasm_projected_range(
                    projected.map(|value| value.origins.as_slice()),
                    source_id.clone(),
                    reference.range,
                );
                AttributeReferenceQuery {
                    source_id: reference_source_id.clone(),
                    range,
                    name_range: wasm_projected_range(
                        projected.map(|value| value.name_origins.as_slice()),
                        reference_source_id,
                        reference.name_range,
                    )
                    .1,
                    name: reference.name.clone(),
                    binding_id: reference.binding_id.map(|id| id.get()),
                    effective_value,
                    error,
                }
            })
            .collect(),
    }
}

fn wasm_project_document_attribute_occurrence(
    output: &mut DocumentAttributeOccurrence,
    projected: &adocweave::preprocess::ProjectedDocumentAttribute,
) {
    output.range = wasm_first_origin_range(&projected.origins, output.range);
    output.name_range = wasm_first_origin_range(&projected.name_origins, output.name_range);
    output.value.source_range =
        wasm_first_origin_range(&projected.value_origins, output.value.source_range);
    for (line, projected_line) in output.value.lines.iter_mut().zip(&projected.value_lines) {
        line.range = wasm_first_origin_range(&projected_line.origins, line.range);
        line.indent_range =
            wasm_first_origin_range(&projected_line.indent_origins, line.indent_range);
        line.content_range =
            wasm_first_origin_range(&projected_line.content_origins, line.content_range);
        line.ending_range =
            wasm_first_origin_range(&projected_line.ending_origins, line.ending_range);
        if let Some(continuation) = &mut line.continuation {
            continuation.range =
                wasm_first_origin_range(&projected_line.continuation_origins, continuation.range);
        }
    }
}

fn wasm_first_origin_range(
    origins: &[adocweave::preprocess::SourceOrigin],
    fallback: TextRange,
) -> TextRange {
    origins.first().map_or(fallback, |origin| TextRange {
        start: origin.range.start().to_u32(),
        end: origin.range.end().to_u32(),
    })
}

fn wasm_projected_range(
    origins: Option<&[adocweave::preprocess::SourceOrigin]>,
    fallback_source_id: Option<String>,
    fallback_range: adocweave::text::TextRange,
) -> (Option<String>, TextRange) {
    origins.and_then(|origins| origins.first()).map_or_else(
        || (fallback_source_id, wasm_text_range(fallback_range)),
        |origin| {
            (
                origin
                    .source_id
                    .as_ref()
                    .map(|source_id| source_id.as_str().to_owned()),
                wasm_text_range(origin.range.text_range()),
            )
        },
    )
}

fn wasm_attribute_resolution(
    value: Result<Option<&str>, adocweave::semantic::AttributeExpansionError>,
) -> (Option<String>, Option<AttributeExpansionError>) {
    match value {
        Ok(value) => (value.map(str::to_owned), None),
        Err(error) => (
            None,
            Some(match error {
                adocweave::semantic::AttributeExpansionError::Undefined => {
                    AttributeExpansionError::Undefined
                }
                adocweave::semantic::AttributeExpansionError::Cycle => {
                    AttributeExpansionError::Cycle
                }
                adocweave::semantic::AttributeExpansionError::DepthLimitExceeded => {
                    AttributeExpansionError::DepthLimitExceeded
                }
                adocweave::semantic::AttributeExpansionError::SizeLimitExceeded => {
                    AttributeExpansionError::SizeLimitExceeded
                }
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_limit_uses_the_public_error_code() {
        let response = AnalyzeResult {
            html: Some("text".to_owned()),
            ..AnalyzeResult::default()
        };
        assert_eq!(
            enforce_output_limit(&response, 1)
                .expect_err("response exceeds one byte")
                .code,
            "output-limit-exceeded"
        );
    }
}
