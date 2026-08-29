//! Analysis-independent render-input validation.

use crate::AdocWeaveError;
use crate::render_input_wire::MAX_SAFE_INTEGER;
use crate::render_input_wire::{CitationOutcome, ReferenceOutcome, RenderInputs, ResourceOutcome};

/// Render inputs whose count and allocation limits were validated.
///
/// The inner wire value is private so meaning conversion cannot consume an
/// unnormalized public value.
#[derive(Debug)]
pub(crate) struct NormalizedRenderInputs(RenderInputs);

pub(crate) fn normalize(
    inputs: RenderInputs,
    analysis_limits: &adocweave::AnalysisLimits,
    output_limits: &adocweave::OutputLimits,
) -> Result<NormalizedRenderInputs, AdocWeaveError> {
    let count = inputs.references.len() as u64
        + inputs.resources.len() as u64
        + inputs.citations.len() as u64
        + inputs
            .generated_bibliography
            .as_ref()
            .map_or(0, |bibliography| bibliography.entries.len() as u64);
    if count > u64::from(analysis_limits.max_references) {
        return Err(limit_error("render input count"));
    }
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
    let bytes = reference_bytes
        .chain(resource_bytes)
        .chain(citation_bytes)
        .chain(bibliography_bytes)
        .fold(0_u64, u64::saturating_add);
    if bytes > u64::from(output_limits.max_output_bytes) {
        return Err(limit_error("render input bytes"));
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
    use adocweave::OutputLimits;

    use super::*;
    use crate::render_input_wire::{
        GeneratedBibliography, GeneratedBibliographyEntry, ReferenceFailureKind, ReferenceOutcome,
        ResolvedReference, ResolvedResource, ResourceFailureKind, ResourceOutcome,
    };

    fn limits(max_references: u32) -> adocweave::AnalysisLimits {
        adocweave::AnalysisLimits {
            max_references,
            ..adocweave::AnalysisLimits::default()
        }
    }

    fn output_limits(max_output_bytes: u32) -> adocweave::OutputLimits {
        adocweave::OutputLimits { max_output_bytes }
    }

    #[test]
    fn count_limit_accepts_the_exact_boundary_and_rejects_one_more() {
        let reference = ResolvedReference {
            source_start: 0,
            source_end: 0,
            outcome: ReferenceOutcome::Failed {
                kind: ReferenceFailureKind::MissingTarget,
            },
        };
        let inputs = RenderInputs {
            references: vec![reference.clone(), reference.clone()],
            resources: Vec::new(),
            citations: Vec::new(),
            generated_bibliography: None,
        };
        assert!(normalize(inputs, &limits(2), &adocweave::OutputLimits::default()).is_ok());

        let inputs = RenderInputs {
            references: vec![reference.clone(), reference.clone(), reference],
            resources: Vec::new(),
            citations: Vec::new(),
            generated_bibliography: None,
        };
        let error = normalize(inputs, &limits(2), &adocweave::OutputLimits::default()).unwrap_err();
        assert_eq!(error.code, "input-limit-exceeded");
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
        assert!(normalize(inputs("abc"), &limits(1), &output_limits(5)).is_ok());
        let error = normalize(inputs("abcd"), &limits(1), &output_limits(5)).unwrap_err();
        assert_eq!(error.code, "input-limit-exceeded");
    }

    #[test]
    fn bibliography_entries_share_the_count_and_output_budgets() {
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

        assert!(normalize(inputs("x"), &limits(1), &output_limits(4)).is_ok());
        assert_eq!(
            normalize(inputs("xx"), &limits(1), &output_limits(4))
                .expect_err("one extra byte exceeds the output limit")
                .code,
            "input-limit-exceeded"
        );
        assert_eq!(
            normalize(inputs("x"), &limits(0), &output_limits(4))
                .expect_err("one bibliography entry consumes one input slot")
                .code,
            "input-limit-exceeded"
        );
    }

    #[test]
    fn duplicates_and_failed_outcomes_are_preserved() {
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
        let normalized = normalize(
            RenderInputs {
                references: vec![reference.clone(), reference],
                resources: vec![resource.clone(), resource],
                citations: Vec::new(),
                generated_bibliography: None,
            },
            &limits(4),
            &OutputLimits::default(),
        )
        .unwrap()
        .into_wire();

        assert_eq!(normalized.references.len(), 2);
        assert_eq!(normalized.resources.len(), 2);
        assert!(matches!(
            normalized.references[0].outcome,
            ReferenceOutcome::Failed {
                kind: ReferenceFailureKind::MissingAnchor
            }
        ));
        assert!(matches!(
            normalized.resources[0].outcome,
            ResourceOutcome::Failed {
                kind: ResourceFailureKind::PermissionDenied
            }
        ));
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
        assert!(
            normalize(
                inputs(MAX_SAFE_INTEGER),
                &limits(1),
                &OutputLimits::default()
            )
            .is_ok()
        );
        let error = normalize(
            inputs(MAX_SAFE_INTEGER + 1),
            &limits(1),
            &OutputLimits::default(),
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid-request");
    }
}
