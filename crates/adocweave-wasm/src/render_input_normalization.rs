//! Analysis-independent render-input validation.

use std::collections::BTreeMap;

use crate::AdocWeaveError;
use crate::render_input_wire::MAX_SAFE_INTEGER;
use crate::render_input_wire::{CitationOutcome, ReferenceOutcome, RenderInputs, ResourceOutcome};

/// Render inputs whose count and allocation limits were validated.
///
/// The inner wire value is private so meaning conversion cannot consume an
/// unnormalized public value.
#[derive(Debug)]
pub(crate) struct NormalizedRenderInputs(RenderInputs);

const MAX_EXTERNAL_INPUT_ITEMS: u64 = 10_000;

pub(crate) fn normalize(
    inputs: RenderInputs,
    documents: &BTreeMap<String, String>,
    source: &str,
    analysis_limits: &adocweave_core::AnalysisLimits,
) -> Result<NormalizedRenderInputs, AdocWeaveError> {
    validate_ranges(&inputs.references, source, |input| {
        (input.source_start, input.source_end)
    })?;
    validate_ranges(&inputs.resources, source, |input| {
        (input.source_start, input.source_end)
    })?;
    validate_ranges(&inputs.citations, source, |input| {
        (input.source_start, input.source_end)
    })?;

    let reference_children = inputs
        .references
        .iter()
        .map(|input| match &input.outcome {
            ReferenceOutcome::Resolved { notices, .. } => notices.len() as u64,
            ReferenceOutcome::Failed { .. } => 0,
        })
        .fold(0_u64, u64::saturating_add);
    let citation_children = inputs
        .citations
        .iter()
        .map(|input| match &input.outcome {
            CitationOutcome::Resolved { segments } => segments.len() as u64,
            CitationOutcome::Failed { .. } => 0,
        })
        .fold(0_u64, u64::saturating_add);
    let bibliography_items = inputs
        .generated_bibliography
        .as_ref()
        .map_or(0, |bibliography| {
            1_u64.saturating_add(bibliography.entries.len() as u64)
        });
    let count = [
        documents.len() as u64,
        inputs.references.len() as u64,
        reference_children,
        inputs.resources.len() as u64,
        inputs.citations.len() as u64,
        citation_children,
        bibliography_items,
    ]
    .into_iter()
    .fold(0_u64, u64::saturating_add);
    if count > MAX_EXTERNAL_INPUT_ITEMS {
        return Err(limit_error("external input count"));
    }
    let document_bytes = documents
        .iter()
        .map(|(id, text)| id.len() as u64 + text.len() as u64);
    let reference_bytes = inputs.references.iter().map(|input| match &input.outcome {
        ReferenceOutcome::Resolved {
            href, display_text, ..
        } => href.len() as u64 + display_text.as_ref().map_or(0, |text| text.len()) as u64,
        ReferenceOutcome::Failed { .. } => 0,
    });
    let resource_bytes = inputs.resources.iter().map(|input| match &input.outcome {
        ResourceOutcome::Resolved {
            href, media_type, ..
        } => href.len() as u64 + media_type.len() as u64,
        ResourceOutcome::Failed { .. } => 0,
    });
    // A citation carries the text the host wants shown, so its segments count
    // against the same output budget as a resolved href.
    let citation_bytes = inputs.citations.iter().map(|input| match &input.outcome {
        CitationOutcome::Resolved { segments } => segments
            .iter()
            .map(|segment| {
                segment.text.len() as u64 + segment.anchor.as_ref().map_or(0, String::len) as u64
            })
            .fold(0_u64, u64::saturating_add),
        CitationOutcome::Failed { .. } => 0,
    });
    let bibliography_bytes = inputs
        .generated_bibliography
        .iter()
        .flat_map(|bibliography| {
            std::iter::once(bibliography.title.len() as u64).chain(bibliography.entries.iter().map(
                |entry| {
                    entry.citation_key.len() as u64
                        + entry.text.len() as u64
                        + entry.label.as_ref().map_or(0, String::len) as u64
                },
            ))
        });
    let bytes = document_bytes
        .chain(reference_bytes)
        .chain(resource_bytes)
        .chain(citation_bytes)
        .chain(bibliography_bytes)
        .fold(0_u64, u64::saturating_add);
    if bytes > u64::from(analysis_limits.max_input_bytes) {
        return Err(limit_error("external input bytes"));
    }
    if inputs.resources.iter().any(|input| {
        matches!(
            input.outcome,
            ResourceOutcome::Resolved {
                byte_length: Some(value),
                ..
            } if value > MAX_SAFE_INTEGER
        )
    }) {
        return Err(invalid_input());
    }
    Ok(NormalizedRenderInputs(inputs))
}

fn validate_ranges<T>(
    inputs: &[T],
    source: &str,
    range: impl Fn(&T) -> (u32, u32),
) -> Result<(), AdocWeaveError> {
    let mut previous = None;
    for input in inputs {
        let (start, end) = range(input);
        let start = usize::try_from(start).map_err(|_| invalid_input())?;
        let end = usize::try_from(end).map_err(|_| invalid_input())?;
        if start > end || source.get(start..end).is_none() {
            return Err(invalid_input());
        }
        if previous.is_some_and(|(previous_start, previous_end)| {
            start < previous_end || (start == previous_start && end == previous_end)
        }) {
            return Err(invalid_input());
        }
        previous = Some((start, end));
    }
    Ok(())
}

impl NormalizedRenderInputs {
    pub(super) fn into_wire(self) -> RenderInputs {
        self.0
    }
}

fn limit_error(resource: &str) -> AdocWeaveError {
    AdocWeaveError {
        code: "input-limit-exceeded".to_owned(),
        message: format!("{resource} exceeds the configured processing limit"),
    }
}

fn invalid_input() -> AdocWeaveError {
    AdocWeaveError {
        code: "invalid-request".to_owned(),
        message: "render input is invalid".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_input_wire::{
        CitationOutcome, GeneratedBibliography, GeneratedBibliographyEntry, ReferenceFailureKind,
        ReferenceNotice, ReferenceOutcome, ResolvedCitation, ResolvedReference, ResolvedResource,
        ResourceFailureKind, ResourceOutcome,
    };

    fn limits(max_input_bytes: u32) -> adocweave_core::AnalysisLimits {
        adocweave_core::AnalysisLimits {
            max_input_bytes,
            ..adocweave_core::AnalysisLimits::default()
        }
    }

    fn normalize_inputs(
        inputs: RenderInputs,
        documents: &BTreeMap<String, String>,
        source: &str,
        max_input_bytes: u32,
    ) -> Result<NormalizedRenderInputs, AdocWeaveError> {
        normalize(inputs, documents, source, &limits(max_input_bytes))
    }

    #[test]
    fn document_count_limit_accepts_ten_thousand_and_rejects_one_more() {
        let documents = (0..10_000)
            .map(|index| (format!("{index}.adoc"), String::new()))
            .collect::<BTreeMap<_, _>>();
        assert!(normalize_inputs(RenderInputs::default(), &documents, "", u32::MAX).is_ok());

        let mut documents = documents;
        documents.insert("overflow.adoc".to_owned(), String::new());
        let error = normalize_inputs(RenderInputs::default(), &documents, "", u32::MAX)
            .expect_err("10,001 documents exceed the fixed item limit");
        assert_eq!(error.code, "input-limit-exceeded");
    }

    #[test]
    fn nested_elements_count_toward_the_fixed_item_limit() {
        let inputs = RenderInputs {
            references: vec![ResolvedReference {
                source_start: 0,
                source_end: 0,
                outcome: ReferenceOutcome::Resolved {
                    href: String::new(),
                    display_text: None,
                    notices: vec![ReferenceNotice::Fallback; 10_000],
                },
            }],
            ..RenderInputs::default()
        };
        let error = normalize_inputs(inputs, &BTreeMap::new(), "", u32::MAX)
            .expect_err("the outer reference and every notice consume input items");
        assert_eq!(error.code, "input-limit-exceeded");
    }

    #[test]
    fn each_resolved_input_array_rejects_ten_thousand_and_one_entries() {
        let source = "x".repeat(10_001);
        let references = (0..10_001)
            .map(|index| ResolvedReference {
                source_start: index,
                source_end: index,
                outcome: ReferenceOutcome::Failed {
                    kind: ReferenceFailureKind::MissingTarget,
                },
            })
            .collect();
        let resources = (0..10_001)
            .map(|index| ResolvedResource {
                source_start: index,
                source_end: index,
                outcome: ResourceOutcome::Failed {
                    kind: ResourceFailureKind::Missing,
                },
            })
            .collect();
        let citations = (0..10_001)
            .map(|index| ResolvedCitation {
                source_start: index,
                source_end: index,
                outcome: CitationOutcome::Failed {
                    kind: ReferenceFailureKind::MissingTarget,
                },
            })
            .collect();
        for inputs in [
            RenderInputs {
                references,
                ..RenderInputs::default()
            },
            RenderInputs {
                resources,
                ..RenderInputs::default()
            },
            RenderInputs {
                citations,
                ..RenderInputs::default()
            },
        ] {
            assert_eq!(
                normalize_inputs(inputs, &BTreeMap::new(), &source, u32::MAX)
                    .expect_err("10,001 resolved inputs exceed the fixed item limit")
                    .code,
                "input-limit-exceeded"
            );
        }
    }

    #[test]
    fn byte_limit_accepts_the_exact_boundary_and_rejects_one_more() {
        let inputs = |href: &str| RenderInputs {
            references: vec![ResolvedReference {
                source_start: 0,
                source_end: 0,
                outcome: ReferenceOutcome::Resolved {
                    href: href.to_owned(),
                    display_text: Some("xy".to_owned()),
                    notices: Vec::new(),
                },
            }],
            resources: Vec::new(),
            citations: Vec::new(),
            generated_bibliography: None,
        };
        assert!(normalize_inputs(inputs("abc"), &BTreeMap::new(), "", 5).is_ok());
        let error = normalize_inputs(inputs("abcd"), &BTreeMap::new(), "", 5).unwrap_err();
        assert_eq!(error.code, "input-limit-exceeded");
    }

    #[test]
    fn document_ids_and_text_share_the_external_byte_limit() {
        let documents = BTreeMap::from([("a".to_owned(), "1234".to_owned())]);
        assert!(normalize_inputs(RenderInputs::default(), &documents, "", 5).is_ok());
        assert_eq!(
            normalize_inputs(RenderInputs::default(), &documents, "", 4)
                .expect_err("the document ID and text both consume bytes")
                .code,
            "input-limit-exceeded"
        );
    }

    #[test]
    fn bibliography_entries_share_the_external_input_budget() {
        let inputs = |text: &str| RenderInputs {
            generated_bibliography: Some(GeneratedBibliography {
                title: "R".to_owned(),
                entries: vec![GeneratedBibliographyEntry {
                    citation_key: "k".to_owned(),
                    text: text.to_owned(),
                    label: Some("l".to_owned()),
                    number: Some(1),
                }],
            }),
            ..RenderInputs::default()
        };

        assert!(normalize_inputs(inputs("x"), &BTreeMap::new(), "", 4).is_ok());
        assert_eq!(
            normalize_inputs(inputs("xx"), &BTreeMap::new(), "", 4)
                .expect_err("one extra byte exceeds the input limit")
                .code,
            "input-limit-exceeded"
        );
    }

    #[test]
    fn duplicate_overlapping_and_unstable_ranges_are_rejected_per_input_kind() {
        let reference = ResolvedReference {
            source_start: 1,
            source_end: 2,
            outcome: ReferenceOutcome::Failed {
                kind: ReferenceFailureKind::MissingAnchor,
            },
        };
        let resource = ResolvedResource {
            source_start: 3,
            source_end: 4,
            outcome: ResourceOutcome::Failed {
                kind: ResourceFailureKind::PermissionDenied,
            },
        };
        let citation = ResolvedCitation {
            source_start: 0,
            source_end: 2,
            outcome: CitationOutcome::Failed {
                kind: ReferenceFailureKind::MissingTarget,
            },
        };
        for inputs in [
            RenderInputs {
                references: vec![reference.clone(), reference],
                ..RenderInputs::default()
            },
            RenderInputs {
                resources: vec![resource.clone(), resource],
                ..RenderInputs::default()
            },
            RenderInputs {
                citations: vec![citation.clone(), citation],
                ..RenderInputs::default()
            },
        ] {
            assert_eq!(
                normalize_inputs(inputs, &BTreeMap::new(), "abcd", u32::MAX)
                    .expect_err("duplicate ranges conflict")
                    .code,
                "invalid-request"
            );
        }

        let overlapping = RenderInputs {
            references: vec![
                ResolvedReference {
                    source_start: 0,
                    source_end: 3,
                    outcome: ReferenceOutcome::Failed {
                        kind: ReferenceFailureKind::MissingTarget,
                    },
                },
                ResolvedReference {
                    source_start: 2,
                    source_end: 4,
                    outcome: ReferenceOutcome::Failed {
                        kind: ReferenceFailureKind::MissingAnchor,
                    },
                },
            ],
            ..RenderInputs::default()
        };
        assert_eq!(
            normalize_inputs(overlapping, &BTreeMap::new(), "abcd", u32::MAX)
                .expect_err("overlapping ranges conflict")
                .code,
            "invalid-request"
        );

        let unstable = RenderInputs {
            references: vec![
                ResolvedReference {
                    source_start: 3,
                    source_end: 4,
                    outcome: ReferenceOutcome::Failed {
                        kind: ReferenceFailureKind::MissingTarget,
                    },
                },
                ResolvedReference {
                    source_start: 0,
                    source_end: 1,
                    outcome: ReferenceOutcome::Failed {
                        kind: ReferenceFailureKind::MissingAnchor,
                    },
                },
            ],
            ..RenderInputs::default()
        };
        assert_eq!(
            normalize_inputs(unstable, &BTreeMap::new(), "abcd", u32::MAX)
                .expect_err("ranges must use stable source order")
                .code,
            "invalid-request"
        );
    }

    #[test]
    fn resource_byte_length_rejects_values_beyond_the_schema_safe_integer() {
        let inputs = |byte_length| RenderInputs {
            references: Vec::new(),
            resources: vec![ResolvedResource {
                source_start: 0,
                source_end: 0,
                outcome: ResourceOutcome::Resolved {
                    href: String::new(),
                    media_type: "image/png".to_owned(),
                    byte_length: Some(byte_length),
                },
            }],
            citations: Vec::new(),
            generated_bibliography: None,
        };
        assert!(normalize_inputs(inputs(MAX_SAFE_INTEGER), &BTreeMap::new(), "", u32::MAX).is_ok());
        let error = normalize_inputs(inputs(MAX_SAFE_INTEGER + 1), &BTreeMap::new(), "", u32::MAX)
            .unwrap_err();
        assert_eq!(error.code, "invalid-request");
    }
}
