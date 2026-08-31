//! Diagnostics and safe source edits shared by all front ends.

use std::error::Error;
use std::fmt;

use serde::Serialize;

use crate::source::TextRange;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DiagnosticId(String);

impl DiagnosticId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DiagnosticCode(String);

impl DiagnosticCode {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "information",
            Self::Hint => "hint",
        }
    }
}

impl Applicability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Maybe => "maybe",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RelatedInformation {
    #[serde(serialize_with = "serialize_range")]
    pub range: TextRange,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TextEdit {
    #[serde(serialize_with = "serialize_range")]
    pub range: TextRange,
    pub replacement: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Applicability {
    Always,
    Maybe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Fix {
    pub title: String,
    pub applicability: Applicability,
    edits: Vec<TextEdit>,
}

impl Fix {
    pub fn new(
        title: impl Into<String>,
        applicability: Applicability,
        mut edits: Vec<TextEdit>,
    ) -> Result<Self, EditConflict> {
        edits.sort_by_key(|edit| (edit.range.start(), edit.range.end()));
        validate_edits(&edits)?;

        Ok(Self {
            title: title.into(),
            applicability,
            edits,
        })
    }

    pub fn edits(&self) -> &[TextEdit] {
        &self.edits
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditConflictKind {
    Duplicate,
    Overlap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditConflict {
    pub first: TextRange,
    pub second: TextRange,
    pub kind: EditConflictKind,
}

impl fmt::Display for EditConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} text edits at {:?} and {:?}",
            self.kind, self.first, self.second
        )
    }
}

impl Error for EditConflict {}

fn validate_edits(edits: &[TextEdit]) -> Result<(), EditConflict> {
    for pair in edits.windows(2) {
        let first = pair[0].range;
        let second = pair[1].range;
        if first == second {
            return Err(EditConflict {
                first,
                second,
                kind: EditConflictKind::Duplicate,
            });
        }
        if first.end() > second.start() {
            return Err(EditConflict {
                first,
                second,
                kind: EditConflictKind::Overlap,
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    pub id: DiagnosticId,
    pub code: DiagnosticCode,
    pub severity: Severity,
    #[serde(serialize_with = "serialize_range")]
    pub range: TextRange,
    pub message: String,
    pub related: Vec<RelatedInformation>,
    pub fixes: Vec<Fix>,
}

/// Sorts diagnostics into the canonical order used by every front end.
pub fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|left, right| {
        (
            left.range.start(),
            left.range.end(),
            &left.code,
            &left.id,
            left.severity,
            &left.message,
        )
            .cmp(&(
                right.range.start(),
                right.range.end(),
                &right.code,
                &right.id,
                right.severity,
                &right.message,
            ))
    });
}

/// Stable failure categories shared by CLI, LSP, Web API, and WASM adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreErrorCode {
    InvalidInput,
    ParseFailed,
    LimitExceeded,
    Cancelled,
    InternalInvariant,
}

impl CoreErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid-input",
            Self::ParseFailed => "parse-failed",
            Self::LimitExceeded => "limit-exceeded",
            Self::Cancelled => "cancelled",
            Self::InternalInvariant => "internal-invariant",
        }
    }
}

fn serialize_range<S>(range: &TextRange, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    #[derive(Serialize)]
    struct Range {
        start: u32,
        end: u32,
    }

    Range {
        start: range.start().to_u32(),
        end: range.end().to_u32(),
    }
    .serialize(serializer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::TextSize;

    fn size(value: usize) -> TextSize {
        TextSize::new(value).expect("small test offset")
    }

    fn range(start: usize, end: usize) -> TextRange {
        TextRange::new(size(start), size(end)).expect("ordered test range")
    }

    fn diagnostic(id: &str, code: &str, severity: Severity, range: TextRange) -> Diagnostic {
        Diagnostic {
            id: DiagnosticId::new(id),
            code: DiagnosticCode::new(code),
            severity,
            message: format!("message for {id}"),
            range,
            related: Vec::new(),
            fixes: Vec::new(),
        }
    }

    #[test]
    fn diagnostic_sort_is_stable_and_canonical() {
        let mut diagnostics = vec![
            diagnostic("b", "z-code", Severity::Warning, range(4, 5)),
            diagnostic("c", "a-code", Severity::Error, range(1, 3)),
            diagnostic("a", "a-code", Severity::Warning, range(1, 2)),
        ];

        sort_diagnostics(&mut diagnostics);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "c", "b"]
        );
    }

    #[test]
    fn diagnostic_fix_sorts_non_conflicting_edits() {
        let fix = Fix::new(
            "fix whitespace",
            Applicability::Always,
            vec![
                TextEdit {
                    range: range(5, 6),
                    replacement: String::new(),
                },
                TextEdit {
                    range: range(1, 2),
                    replacement: " ".to_owned(),
                },
            ],
        )
        .expect("non-overlapping edits");

        assert_eq!(fix.edits()[0].range, range(1, 2));
        assert_eq!(fix.edits()[1].range, range(5, 6));
    }

    #[test]
    fn diagnostic_fix_rejects_duplicate_and_overlapping_edits() {
        let duplicate = vec![
            TextEdit {
                range: range(1, 2),
                replacement: "a".to_owned(),
            },
            TextEdit {
                range: range(1, 2),
                replacement: "b".to_owned(),
            },
        ];
        assert_eq!(
            Fix::new("duplicate", Applicability::Always, duplicate),
            Err(EditConflict {
                first: range(1, 2),
                second: range(1, 2),
                kind: EditConflictKind::Duplicate,
            })
        );

        let overlapping = vec![
            TextEdit {
                range: range(1, 4),
                replacement: String::new(),
            },
            TextEdit {
                range: range(3, 5),
                replacement: String::new(),
            },
        ];
        assert_eq!(
            Fix::new("overlap", Applicability::Maybe, overlapping),
            Err(EditConflict {
                first: range(1, 4),
                second: range(3, 5),
                kind: EditConflictKind::Overlap,
            })
        );
    }

    #[test]
    fn diagnostic_core_error_codes_are_stable() {
        assert_eq!(CoreErrorCode::InvalidInput.as_str(), "invalid-input");
        assert_eq!(CoreErrorCode::ParseFailed.as_str(), "parse-failed");
        assert_eq!(CoreErrorCode::LimitExceeded.as_str(), "limit-exceeded");
        assert_eq!(CoreErrorCode::Cancelled.as_str(), "cancelled");
        assert_eq!(
            CoreErrorCode::InternalInvariant.as_str(),
            "internal-invariant"
        );
    }
}
