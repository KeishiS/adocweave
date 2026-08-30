//! Analysis-dependent conversion from normalized render inputs to core values.

use adocweave_core::Analysis;

use crate::AdocWeaveError;
use crate::render_input_normalization::NormalizedRenderInputs;
use crate::render_input_wire::{
    CitationOutcome, ReferenceFailureKind, ReferenceNotice, ReferenceOutcome, ResourceFailureKind,
    ResourceOutcome,
};

pub(crate) fn convert(
    inputs: NormalizedRenderInputs,
    analysis: &Analysis,
) -> Result<adocweave_core::resolution::RenderInputs, AdocWeaveError> {
    let inputs = inputs.into_wire();
    let references = inputs
        .references
        .into_iter()
        .map(|resolution| {
            let range = source_range(resolution.source_start, resolution.source_end, analysis)?;
            Ok(match resolution.outcome {
                ReferenceOutcome::Resolved {
                    href,
                    display_text,
                    notices,
                } => {
                    let mut resolved =
                        adocweave_core::resolution::ResolvedReference::resolved(range, href)
                            .with_notices(
                                notices
                                    .into_iter()
                                    .map(|notice| {
                                        adocweave_core::resolution::ResolutionNotice {
                                kind: match notice {
                                    ReferenceNotice::Fallback => {
                                        adocweave_core::resolution::ResolutionNoticeKind::Fallback
                                    }
                                },
                            }
                                    })
                                    .collect(),
                            );
                    if let Some(display_text) = display_text {
                        resolved = resolved.with_display_text(display_text);
                    }
                    resolved
                }
                ReferenceOutcome::Failed { kind } => {
                    adocweave_core::resolution::ResolvedReference::failed(
                        range,
                        reference_failure(kind),
                    )
                }
            })
        })
        .collect::<Result<Vec<_>, AdocWeaveError>>()?;
    let resources = inputs
        .resources
        .into_iter()
        .map(|resolution| {
            let range = source_range(resolution.source_start, resolution.source_end, analysis)?;
            Ok(match resolution.outcome {
                ResourceOutcome::Resolved {
                    href,
                    media_type,
                    byte_length,
                } => adocweave_core::resolution::ResolvedResource::resolved(
                    range,
                    href,
                    adocweave_core::resolution::MediaType::parse(&media_type)
                        .map_err(|_| invalid_input())?,
                    byte_length,
                ),
                ResourceOutcome::Failed { kind } => {
                    adocweave_core::resolution::ResolvedResource::failed(
                        range,
                        adocweave_core::resolution::ResourceFailure {
                            kind: match kind {
                                ResourceFailureKind::Missing => {
                                    adocweave_core::resolution::ResourceFailureKind::Missing
                                }
                                ResourceFailureKind::OutsideRoot => {
                                    adocweave_core::resolution::ResourceFailureKind::OutsideRoot
                                }
                                ResourceFailureKind::SchemeDenied => {
                                    adocweave_core::resolution::ResourceFailureKind::SchemeDenied
                                }
                                ResourceFailureKind::PermissionDenied => {
                                    adocweave_core::resolution::ResourceFailureKind::PermissionDenied
                                }
                                ResourceFailureKind::MediaTypeUnavailable => {
                                    adocweave_core::resolution::ResourceFailureKind::MediaTypeUnavailable
                                }
                                ResourceFailureKind::ResolverFailure => {
                                    adocweave_core::resolution::ResourceFailureKind::ResolverFailure
                                }
                            },
                        },
                    )
                }
            })
        })
        .collect::<Result<Vec<_>, AdocWeaveError>>()?;
    let citations = inputs
        .citations
        .into_iter()
        .map(|resolution| {
            let range = source_range(resolution.source_start, resolution.source_end, analysis)?;
            Ok(match resolution.outcome {
                CitationOutcome::Resolved { segments } => {
                    adocweave_core::resolution::ResolvedCitation::resolved(
                        range,
                        segments
                            .into_iter()
                            .map(|segment| adocweave_core::resolution::CitationSegment {
                                text: segment.text,
                                anchor: segment.anchor,
                            })
                            .collect(),
                    )
                }
                CitationOutcome::Failed { kind } => {
                    adocweave_core::resolution::ResolvedCitation::failed(
                        range,
                        reference_failure(kind),
                    )
                }
            })
        })
        .collect::<Result<Vec<_>, AdocWeaveError>>()?;
    let generated_bibliography = inputs.generated_bibliography.map(|bibliography| {
        adocweave_core::resolution::GeneratedBibliography::new(
            bibliography.title,
            bibliography
                .entries
                .into_iter()
                .map(|entry| {
                    let generated = adocweave_core::resolution::GeneratedBibliographyEntry::new(
                        entry.citation_key,
                        entry.text,
                    );
                    let generated = match entry.label {
                        Some(label) => generated.with_label(label),
                        None => generated,
                    };
                    match entry.number {
                        Some(number) => generated.with_number(number),
                        None => generated,
                    }
                })
                .collect(),
        )
    });
    let inputs = adocweave_core::resolution::RenderInputs::default()
        .with_references(references)
        .with_resources(resources)
        .with_citations(citations);
    Ok(match generated_bibliography {
        Some(bibliography) => inputs.with_generated_bibliography(bibliography),
        None => inputs,
    })
}

/// Maps a wire failure kind to the core kind shared by references and citations.
fn reference_failure(kind: ReferenceFailureKind) -> adocweave_core::resolution::ResolverFailure {
    adocweave_core::resolution::ResolverFailure {
        kind: match kind {
            ReferenceFailureKind::MissingTarget => {
                adocweave_core::resolution::ResolutionFailureKind::MissingTarget
            }
            ReferenceFailureKind::MissingAnchor => {
                adocweave_core::resolution::ResolutionFailureKind::MissingAnchor
            }
            ReferenceFailureKind::AmbiguousTarget => {
                adocweave_core::resolution::ResolutionFailureKind::AmbiguousTarget
            }
            ReferenceFailureKind::OutsideRoot => {
                adocweave_core::resolution::ResolutionFailureKind::OutsideRoot
            }
            ReferenceFailureKind::ResolverFailure => {
                adocweave_core::resolution::ResolutionFailureKind::ResolverFailure
            }
        },
    }
}

fn source_range(
    start: u32,
    end: u32,
    analysis: &Analysis,
) -> Result<adocweave_core::text::TextRange, AdocWeaveError> {
    let start = adocweave_core::text::TextSize::new(start as usize).map_err(|_| invalid_input())?;
    let end = adocweave_core::text::TextSize::new(end as usize).map_err(|_| invalid_input())?;
    let range = adocweave_core::text::TextRange::new(start, end).map_err(|_| invalid_input())?;
    analysis
        .source_document()
        .text(range)
        .ok_or_else(invalid_input)?;
    Ok(range)
}

fn invalid_input() -> AdocWeaveError {
    AdocWeaveError {
        code: "invalid-request".to_owned(),
        message: "resolved resource input is invalid".to_owned(),
    }
}
