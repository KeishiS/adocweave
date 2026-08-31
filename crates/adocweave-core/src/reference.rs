//! Typed queries and owned results for cross-document and scheme references.
//!
//! Parsing exposes queries without performing I/O. Hosts resolve those queries and
//! pass validated results to consumers as owned values.

use crate::core::SourceId;
use crate::source::TextRange;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReferenceKey {
    Local {
        anchor: String,
    },
    Document {
        document: String,
        anchor: Option<String>,
    },
    Scheme {
        scheme: String,
        locator: String,
        anchor: Option<String>,
    },
}

impl ReferenceKey {
    pub fn parse(target: &str) -> Option<Self> {
        if let Some(anchor) = target.strip_prefix('#') {
            return (!anchor.is_empty()).then(|| Self::Local {
                anchor: anchor.to_owned(),
            });
        }
        if let Some(colon) = target.find(':') {
            let scheme = &target[..colon];
            if scheme.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'+' | b'-' | b'.'))
            }) {
                let remainder = &target[colon + 1..];
                let (locator, anchor) = remainder
                    .split_once('#')
                    .map_or((remainder, None), |(locator, anchor)| {
                        (locator, Some(anchor))
                    });
                return Some(Self::Scheme {
                    scheme: scheme.to_ascii_lowercase(),
                    locator: locator.to_owned(),
                    anchor: anchor.map(str::to_owned),
                });
            }
        }
        let (document, anchor) = target
            .split_once('#')
            .map_or((target, None), |(document, anchor)| {
                (document, Some(anchor))
            });
        (!document.is_empty()).then(|| Self::Document {
            document: document.to_owned(),
            anchor: anchor.map(str::to_owned),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceQuery {
    pub source_id: Option<SourceId>,
    pub source_range: TextRange,
    pub target: ReferenceKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionFailureKind {
    MissingTarget,
    MissingAnchor,
    AmbiguousTarget,
    OutsideRoot,
    ResolverFailure,
}

impl ResolutionFailureKind {
    pub const fn diagnostic_code(self) -> &'static str {
        match self {
            Self::MissingTarget => "missing-reference-target",
            Self::MissingAnchor => "missing-reference-anchor",
            Self::AmbiguousTarget => "ambiguous-reference-target",
            Self::OutsideRoot => "reference-outside-root",
            Self::ResolverFailure => "reference-resolver-failure",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolverFailure {
    pub kind: ResolutionFailureKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedReference {
    pub source_range: TextRange,
    pub outcome: ResolutionOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolutionOutcome {
    Resolved {
        href: String,
        display_text: Option<String>,
        notices: Vec<ResolutionNotice>,
    },
    Failed(ResolverFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionNoticeKind {
    Fallback,
}

impl ResolutionNoticeKind {
    pub const fn diagnostic_code(self) -> &'static str {
        match self {
            Self::Fallback => "reference-resolution-fallback",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionNotice {
    pub kind: ResolutionNoticeKind,
}

impl ResolvedReference {
    pub fn resolved(source_range: TextRange, href: impl Into<String>) -> Self {
        Self {
            source_range,
            outcome: ResolutionOutcome::Resolved {
                href: href.into(),
                display_text: None,
                notices: Vec::new(),
            },
        }
    }

    pub fn with_display_text(mut self, display_text: impl Into<String>) -> Self {
        if let ResolutionOutcome::Resolved {
            display_text: current,
            ..
        } = &mut self.outcome
        {
            *current = Some(display_text.into());
        }
        self
    }

    pub fn with_notices(mut self, notices: Vec<ResolutionNotice>) -> Self {
        if let ResolutionOutcome::Resolved {
            notices: current, ..
        } = &mut self.outcome
        {
            *current = notices;
        }
        self
    }

    pub fn failed(source_range: TextRange, failure: ResolverFailure) -> Self {
        Self {
            source_range,
            outcome: ResolutionOutcome::Failed(failure),
        }
    }
}

pub fn query_from_reference(
    source_id: Option<SourceId>,
    reference: &crate::inline_model::Reference,
) -> Option<ReferenceQuery> {
    let target = reference.target.clone()?;
    Some(ReferenceQuery {
        source_id,
        source_range: reference.range,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::ResolutionFailureKind;

    #[test]
    fn resolver_contract_exposes_stable_failure_codes() {
        assert_eq!(
            ResolutionFailureKind::MissingAnchor.diagnostic_code(),
            "missing-reference-anchor"
        );
        assert_eq!(
            ResolutionFailureKind::ResolverFailure.diagnostic_code(),
            "reference-resolver-failure"
        );
    }
}
