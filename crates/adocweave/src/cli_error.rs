//! Command-line error categories and the exit status each reports.

use std::error::Error;
use std::fmt;
use std::io;

use adocweave_core::preprocess::PreprocessErrorKind;
use adocweave_project::{
    ProjectExpansionError, ProjectLimit, ProjectParseError, ProjectResourceErrorCode,
    ProjectTargetError,
};

use crate::exit_code::CliExitCode;
use crate::{commands, preview};

#[derive(Debug)]
pub(crate) enum CliError {
    Arguments(clap::Error),
    Usage(String),
    Read {
        source_name: String,
        source: io::Error,
    },
    Write(io::Error),
    InvalidUtf8 {
        valid_up_to: usize,
    },
    Position(adocweave_core::text::PositionError),
    OutputLimit {
        limit: u32,
        actual: u64,
    },
    FormattingRequired,
    Stylesheet(String),
    Path(String),
    PartialWrite {
        files: usize,
        changed: usize,
        updated: usize,
        unchanged: usize,
        failed: usize,
    },
    Preview(preview::Error),
    LanguageServer(adocweave_lsp::StdioError),
    Project(adocweave_project::ProjectError),
    ProjectLimit(adocweave_project::ProjectLimit),
    ProjectPrimary(adocweave_project::ProjectTargetError),
    ProjectTarget(adocweave_project::ProjectTargetError),
    ProjectExpansion(adocweave_project::ProjectExpansionError),
    Serialize(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arguments(source) => source.fmt(formatter),
            Self::Usage(message) => formatter.write_str(message),
            Self::Read {
                source_name,
                source,
            } => write!(formatter, "could not read {source_name}: {source}"),
            Self::Write(source) => write!(formatter, "could not write output: {source}"),
            Self::InvalidUtf8 { valid_up_to } => write!(
                formatter,
                "input is not valid UTF-8 (invalid byte starts at offset {valid_up_to})"
            ),
            Self::Position(source) => source.fmt(formatter),
            Self::OutputLimit { limit, actual } => {
                write!(
                    formatter,
                    "output bytes limit exceeded (limit {limit}, actual {actual})"
                )
            }
            Self::FormattingRequired => formatter.write_str("document is not formatted"),
            Self::Stylesheet(message) => formatter.write_str(message),
            Self::Path(message) => formatter.write_str(message),
            Self::PartialWrite {
                files,
                changed,
                updated,
                unchanged,
                failed,
            } => write!(
                formatter,
                "file updates completed with failures: files={files}, changed={changed}, updated={updated}, unchanged={unchanged}, failed={failed}"
            ),
            Self::Preview(source) => source.fmt(formatter),
            Self::LanguageServer(source) => source.fmt(formatter),
            Self::Project(adocweave_project::ProjectError::Limit(ProjectLimit::Files {
                limit,
            })) => write!(
                formatter,
                "filesystem resource count limit exceeded: {limit}"
            ),
            Self::Project(adocweave_project::ProjectError::Limit(ProjectLimit::ReadBytes {
                ..
            })) => formatter.write_str("analysis snapshot total byte limit exceeded"),
            Self::Project(source) => source.fmt(formatter),
            Self::ProjectLimit(ProjectLimit::Files { limit }) => {
                write!(formatter, "filesystem file limit exceeded: {limit}")
            }
            Self::ProjectLimit(ProjectLimit::ResourceBytes { .. }) => {
                formatter.write_str("analysis snapshot single-resource byte limit exceeded")
            }
            Self::ProjectLimit(ProjectLimit::ReadBytes { .. }) => {
                formatter.write_str("analysis snapshot total byte limit exceeded")
            }
            Self::ProjectLimit(source) => source.fmt(formatter),
            Self::ProjectPrimary(ProjectTargetError::Read(source)) => write!(
                formatter,
                "could not read input: {source}; symbolic links and non-regular files are not accepted"
            ),
            Self::ProjectPrimary(ProjectTargetError::Incomplete(ProjectLimit::ResourceBytes {
                ..
            })) => formatter.write_str("analysis snapshot total byte limit exceeded"),
            Self::ProjectPrimary(ProjectTargetError::Incomplete(ProjectLimit::ReadBytes {
                ..
            })) => formatter.write_str("analysis snapshot total byte limit exceeded"),
            Self::ProjectPrimary(ProjectTargetError::Incomplete(ProjectLimit::Files { limit })) => {
                write!(
                    formatter,
                    "filesystem resource count limit exceeded: {limit}"
                )
            }
            Self::ProjectPrimary(source) => source.fmt(formatter),
            Self::ProjectTarget(ProjectTargetError::Read(source)) => match source.code {
                ProjectResourceErrorCode::Missing => {
                    write!(formatter, "could not read input: {source}")
                }
                ProjectResourceErrorCode::OutsideAuthority => {
                    write!(formatter, "unsafe include target: {source}")
                }
                ProjectResourceErrorCode::ReadFailed => write!(
                    formatter,
                    "could not read input: {source}; symbolic links and non-regular files are not accepted"
                ),
                _ => write!(formatter, "could not read input: {source}"),
            },
            Self::ProjectTarget(ProjectTargetError::Incomplete(limit)) => match limit {
                ProjectLimit::Files { limit } => {
                    write!(formatter, "filesystem file limit exceeded: {limit}")
                }
                ProjectLimit::ResourceBytes { .. } => {
                    formatter.write_str("analysis snapshot single-resource byte limit exceeded")
                }
                ProjectLimit::ReadBytes { .. } => {
                    formatter.write_str("analysis snapshot total byte limit exceeded")
                }
                _ => limit.fmt(formatter),
            },
            Self::ProjectTarget(source) => source.fmt(formatter),
            Self::ProjectExpansion(ProjectExpansionError::Resource(source)) => match source.code {
                ProjectResourceErrorCode::Missing => {
                    write!(formatter, "could not read input: {source}")
                }
                ProjectResourceErrorCode::OutsideAuthority => {
                    write!(formatter, "unsafe include target: {source}")
                }
                ProjectResourceErrorCode::ReadFailed => write!(
                    formatter,
                    "could not read input: {source}; symbolic links and non-regular files are not accepted"
                ),
                _ => write!(formatter, "could not read input: {source}"),
            },
            Self::ProjectExpansion(ProjectExpansionError::Incomplete(limit)) => match limit {
                ProjectLimit::Files { limit } => {
                    write!(formatter, "filesystem file limit exceeded: {limit}")
                }
                ProjectLimit::ResourceBytes { .. } => {
                    formatter.write_str("analysis snapshot single-resource byte limit exceeded")
                }
                ProjectLimit::ReadBytes { .. } => {
                    formatter.write_str("analysis snapshot total byte limit exceeded")
                }
                _ => limit.fmt(formatter),
            },
            Self::ProjectExpansion(source) => source.fmt(formatter),
            Self::Serialize(message) => {
                write!(formatter, "cannot serialize diagnostics: {message}")
            }
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Arguments(source) => Some(source),
            Self::Read { source, .. } | Self::Write(source) => Some(source),
            Self::Position(source) => Some(source),
            Self::Preview(source) => Some(source),
            Self::LanguageServer(source) => Some(source),
            Self::Project(source) => Some(source),
            Self::ProjectLimit(source) => Some(source),
            Self::ProjectPrimary(source) => Some(source),
            Self::ProjectTarget(source) => Some(source),
            Self::ProjectExpansion(source) => Some(source),
            Self::Usage(_)
            | Self::Serialize(_)
            | Self::InvalidUtf8 { .. }
            | Self::OutputLimit { .. }
            | Self::FormattingRequired
            | Self::Stylesheet(_)
            | Self::Path(_)
            | Self::PartialWrite { .. } => None,
        }
    }
}

impl CliError {
    /// Names the category a caller sees as the exit status.
    pub(crate) fn exit_code(&self) -> CliExitCode {
        match self {
            // What the caller asked for cannot be acted on as written.
            Self::Arguments(_) | Self::Usage(_) | Self::Path(_) | Self::Stylesheet(_) => {
                CliExitCode::Usage
            }
            // A file, stream or resource could not be read or written. The input
            // may be fine; the surroundings were not.
            Self::Read { .. } | Self::Write(_) | Self::PartialWrite { .. } | Self::Preview(_) => {
                CliExitCode::InputOutput
            }
            Self::LanguageServer(source) => language_server_exit_code(source.kind()),
            Self::Project(adocweave_project::ProjectError::Config(_))
            | Self::Project(adocweave_project::ProjectError::Authority(_))
            | Self::ProjectPrimary(adocweave_project::ProjectTargetError::Read(_))
            | Self::ProjectTarget(adocweave_project::ProjectTargetError::Read(_))
            | Self::ProjectExpansion(adocweave_project::ProjectExpansionError::Resource(_)) => {
                CliExitCode::InputOutput
            }
            Self::Project(adocweave_project::ProjectError::TargetSelection(_))
            | Self::Project(adocweave_project::ProjectError::InvalidInput(_)) => CliExitCode::Usage,
            // A configured bound was reached, so the work stopped rather than
            // grew without limit.
            Self::OutputLimit { .. }
            | Self::ProjectLimit(_)
            | Self::Project(adocweave_project::ProjectError::Limit(_))
            | Self::ProjectPrimary(adocweave_project::ProjectTargetError::Incomplete(_))
            | Self::ProjectTarget(adocweave_project::ProjectTargetError::Incomplete(_))
            | Self::ProjectExpansion(adocweave_project::ProjectExpansionError::Incomplete(_))
            | Self::ProjectExpansion(adocweave_project::ProjectExpansionError::Projection(_))
            | Self::ProjectPrimary(ProjectTargetError::Parse(ProjectParseError::LimitExceeded {
                ..
            }))
            | Self::ProjectTarget(ProjectTargetError::Parse(ProjectParseError::LimitExceeded {
                ..
            }))
            | Self::ProjectExpansion(ProjectExpansionError::Parse(
                ProjectParseError::LimitExceeded { .. },
            )) => CliExitCode::LimitExceeded,
            Self::ProjectExpansion(ProjectExpansionError::Preprocess(error))
                if is_preprocess_limit(error.kind) =>
            {
                CliExitCode::LimitExceeded
            }
            // The document itself is the problem, which is what a diagnostic
            // reports. Everything left describes the document or the analysis of
            // it, so it shares the status diagnostics use.
            Self::InvalidUtf8 { .. }
            | Self::Position(_)
            | Self::FormattingRequired
            | Self::Serialize(_)
            | Self::Project(adocweave_project::ProjectError::Cancelled)
            | Self::ProjectPrimary(adocweave_project::ProjectTargetError::Parse(_))
            | Self::ProjectPrimary(adocweave_project::ProjectTargetError::EditConflict(_))
            | Self::ProjectTarget(adocweave_project::ProjectTargetError::Parse(_))
            | Self::ProjectTarget(adocweave_project::ProjectTargetError::EditConflict(_))
            | Self::ProjectExpansion(adocweave_project::ProjectExpansionError::Options(_))
            | Self::ProjectExpansion(adocweave_project::ProjectExpansionError::Preprocess(_))
            | Self::ProjectExpansion(adocweave_project::ProjectExpansionError::Parse(_)) => {
                CliExitCode::Diagnostics
            }
        }
    }
}

fn language_server_exit_code(kind: adocweave_lsp::StdioErrorKind) -> CliExitCode {
    match kind {
        adocweave_lsp::StdioErrorKind::Protocol => CliExitCode::Diagnostics,
        adocweave_lsp::StdioErrorKind::Runtime => CliExitCode::InputOutput,
    }
}

fn is_preprocess_limit(kind: PreprocessErrorKind) -> bool {
    matches!(
        kind,
        PreprocessErrorKind::DepthLimit
            | PreprocessErrorKind::IncludeLimit
            | PreprocessErrorKind::ByteLimit
            | PreprocessErrorKind::NodeLimit
            | PreprocessErrorKind::SourceMapLimit
    )
}

pub(crate) fn convert_error(error: commands::convert::Error) -> CliError {
    match error {
        commands::convert::Error::Html(source) => html_policy_error(source),
    }
}

pub(crate) fn check_error(error: commands::check::Error) -> CliError {
    match error {
        commands::check::Error::Position(source) => CliError::Position(source),
        commands::check::Error::Serialize(message) => CliError::Serialize(message),
    }
}

pub(crate) fn format_error(error: commands::format::Error) -> CliError {
    match error {
        commands::format::Error::InvalidUtf8 { valid_up_to } => {
            CliError::InvalidUtf8 { valid_up_to }
        }
        commands::format::Error::Position(source) => CliError::Position(source),
    }
}

pub(crate) fn preview_error(error: commands::preview::Error) -> CliError {
    match error {
        commands::preview::Error::Input(message) => CliError::Path(message),
        commands::preview::Error::Server(source) => CliError::Preview(source),
    }
}

pub(crate) fn html_policy_error(error: commands::html_policy::Error) -> CliError {
    match error {
        commands::html_policy::Error::Read {
            source_name,
            source,
        } => CliError::Read {
            source_name,
            source,
        },
        commands::html_policy::Error::ProjectLimit(limit) => CliError::ProjectLimit(limit),
        commands::html_policy::Error::Stylesheet(message) => CliError::Stylesheet(message),
        commands::html_policy::Error::Usage(message) => CliError::Usage(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_write_reports_counts_and_an_input_output_exit() {
        let error = CliError::PartialWrite {
            files: 3,
            changed: 2,
            updated: 1,
            unchanged: 1,
            failed: 1,
        };

        assert_eq!(error.exit_code(), CliExitCode::InputOutput);
        assert_eq!(
            error.to_string(),
            "file updates completed with failures: files=3, changed=2, updated=1, unchanged=1, failed=1"
        );
    }

    #[test]
    fn preprocessing_limits_are_distinct_from_document_diagnostics() {
        for kind in [
            PreprocessErrorKind::DepthLimit,
            PreprocessErrorKind::IncludeLimit,
            PreprocessErrorKind::ByteLimit,
            PreprocessErrorKind::NodeLimit,
            PreprocessErrorKind::SourceMapLimit,
        ] {
            assert!(is_preprocess_limit(kind), "{kind:?}");
        }
        assert!(!is_preprocess_limit(PreprocessErrorKind::MissingResource));
        assert!(!is_preprocess_limit(PreprocessErrorKind::InvalidDirective));
    }

    #[test]
    fn language_server_failures_map_to_existing_cli_exit_codes() {
        assert_eq!(
            language_server_exit_code(adocweave_lsp::StdioErrorKind::Protocol),
            CliExitCode::Diagnostics
        );
        assert_eq!(
            language_server_exit_code(adocweave_lsp::StdioErrorKind::Runtime),
            CliExitCode::InputOutput
        );
    }
}
