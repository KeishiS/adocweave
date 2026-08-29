//! Analysis-dependent conversion from normalized render inputs to core values.

use adocweave::Analysis;

use crate::AdocWeaveError;
use crate::render_input_normalization::NormalizedRenderInputs;
use crate::render_input_wire::{
    CitationOutcome, ReferenceFailureKind, ReferenceNotice, ReferenceOutcome, ResourceFailureKind,
    ResourceOutcome,
};

pub(crate) fn convert(
    inputs: NormalizedRenderInputs,
    analysis: &Analysis,
) -> Result<adocweave::resolution::RenderInputs, AdocWeaveError> {
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
                        adocweave::resolution::ResolvedReference::resolved(range, href)
                            .with_notices(
                                notices
                                    .into_iter()
                                    .map(|notice| {
                                        adocweave::resolution::ResolutionNotice {
                                    kind: match notice {
                                        ReferenceNotice::Fallback => {
                                            adocweave::resolution::ResolutionNoticeKind::Fallback
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
                    adocweave::resolution::ResolvedReference::failed(range, reference_failure(kind))
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
                } => adocweave::resolution::ResolvedResource::resolved(
                    range,
                    href,
                    adocweave::resolution::MediaType::parse(&media_type)
                        .map_err(|_| invalid_input())?,
                    byte_length,
                ),
                ResourceOutcome::Failed { kind } => {
                    adocweave::resolution::ResolvedResource::failed(
                        range,
                        adocweave::resolution::ResourceFailure {
                            kind: match kind {
                                ResourceFailureKind::Missing => {
                                    adocweave::resolution::ResourceFailureKind::Missing
                                }
                                ResourceFailureKind::OutsideRoot => {
                                    adocweave::resolution::ResourceFailureKind::OutsideRoot
                                }
                                ResourceFailureKind::SchemeDenied => {
                                    adocweave::resolution::ResourceFailureKind::SchemeDenied
                                }
                                ResourceFailureKind::PermissionDenied => {
                                    adocweave::resolution::ResourceFailureKind::PermissionDenied
                                }
                                ResourceFailureKind::MediaTypeUnavailable => {
                                    adocweave::resolution::ResourceFailureKind::MediaTypeUnavailable
                                }
                                ResourceFailureKind::ResolverFailure => {
                                    adocweave::resolution::ResourceFailureKind::ResolverFailure
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
                    adocweave::resolution::ResolvedCitation::resolved(
                        range,
                        segments
                            .into_iter()
                            .map(|segment| adocweave::resolution::CitationSegment {
                                text: segment.text,
                                anchor: segment.anchor,
                            })
                            .collect(),
                    )
                }
                CitationOutcome::Failed { kind } => {
                    adocweave::resolution::ResolvedCitation::failed(range, reference_failure(kind))
                }
            })
        })
        .collect::<Result<Vec<_>, AdocWeaveError>>()?;
    let generated_bibliography = inputs.generated_bibliography.map(|bibliography| {
        adocweave::resolution::GeneratedBibliography::new(
            bibliography.title,
            bibliography
                .entries
                .into_iter()
                .map(|entry| {
                    let generated = adocweave::resolution::GeneratedBibliographyEntry::new(
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
    let inputs = adocweave::resolution::RenderInputs::default()
        .with_references(references)
        .with_resources(resources)
        .with_citations(citations);
    Ok(match generated_bibliography {
        Some(bibliography) => inputs.with_generated_bibliography(bibliography),
        None => inputs,
    })
}

/// Maps a wire failure kind to the core kind shared by references and citations.
fn reference_failure(kind: ReferenceFailureKind) -> adocweave::resolution::ResolverFailure {
    adocweave::resolution::ResolverFailure {
        kind: match kind {
            ReferenceFailureKind::MissingTarget => {
                adocweave::resolution::ResolutionFailureKind::MissingTarget
            }
            ReferenceFailureKind::MissingAnchor => {
                adocweave::resolution::ResolutionFailureKind::MissingAnchor
            }
            ReferenceFailureKind::AmbiguousTarget => {
                adocweave::resolution::ResolutionFailureKind::AmbiguousTarget
            }
            ReferenceFailureKind::OutsideRoot => {
                adocweave::resolution::ResolutionFailureKind::OutsideRoot
            }
            ReferenceFailureKind::ResolverFailure => {
                adocweave::resolution::ResolutionFailureKind::ResolverFailure
            }
        },
    }
}

fn source_range(
    start: u32,
    end: u32,
    analysis: &Analysis,
) -> Result<adocweave::text::TextRange, AdocWeaveError> {
    let start = adocweave::text::TextSize::new(start as usize).map_err(|_| invalid_input())?;
    let end = adocweave::text::TextSize::new(end as usize).map_err(|_| invalid_input())?;
    let range = adocweave::text::TextRange::new(start, end).map_err(|_| invalid_input())?;
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
