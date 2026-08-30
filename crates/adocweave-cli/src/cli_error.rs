//! Command-line error categories and the exit status each reports.

use std::error::Error;
use std::fmt;
use std::io;

use adocweave_host::ExitStatus;
use adocweave_project::{ProjectLimit, ProjectResourceErrorCode, ProjectTargetError};

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
    Position(adocweave::text::PositionError),
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
    ProjectPrimary(adocweave_project::ProjectTargetError),
    ProjectTarget(adocweave_project::ProjectTargetError),
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
            Self::ProjectPrimary(source) => Some(source),
            Self::ProjectTarget(source) => Some(source),
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
    pub(crate) fn exit_status(&self) -> ExitStatus {
        match self {
            // What the caller asked for cannot be acted on as written.
            Self::Arguments(_) | Self::Usage(_) | Self::Path(_) | Self::Stylesheet(_) => {
                ExitStatus::Usage
            }
            // A file, stream or resource could not be read or written. The input
            // may be fine; the surroundings were not.
            Self::Read { .. } | Self::Write(_) | Self::PartialWrite { .. } | Self::Preview(_) => {
                ExitStatus::InputOutput
            }
            Self::LanguageServer(source) => source.exit_status(),
            Self::Project(adocweave_project::ProjectError::Config(_))
            | Self::Project(adocweave_project::ProjectError::Authority(_))
            | Self::ProjectPrimary(adocweave_project::ProjectTargetError::Read(_))
            | Self::ProjectTarget(adocweave_project::ProjectTargetError::Read(_)) => {
                ExitStatus::InputOutput
            }
            Self::Project(adocweave_project::ProjectError::TargetSelection(_))
            | Self::Project(adocweave_project::ProjectError::InvalidInput(_)) => ExitStatus::Usage,
            // A configured bound was reached, so the work stopped rather than
            // grew without limit.
            Self::OutputLimit { .. }
            | Self::Project(adocweave_project::ProjectError::Limit(_))
            | Self::ProjectPrimary(adocweave_project::ProjectTargetError::Incomplete(_))
            | Self::ProjectTarget(adocweave_project::ProjectTargetError::Incomplete(_)) => {
                ExitStatus::LimitExceeded
            }
            // The document itself is the problem, which is what a diagnostic
            // reports. Everything left describes the document or the analysis of
            // it, so it shares the status diagnostics use.
            Self::InvalidUtf8 { .. }
            | Self::Position(_)
            | Self::FormattingRequired
            | Self::Serialize(_)
            | Self::Project(adocweave_project::ProjectError::Cancelled)
            | Self::ProjectPrimary(adocweave_project::ProjectTargetError::Analysis(_))
            | Self::ProjectPrimary(adocweave_project::ProjectTargetError::EditConflict(_))
            | Self::ProjectTarget(adocweave_project::ProjectTargetError::Analysis(_))
            | Self::ProjectTarget(adocweave_project::ProjectTargetError::EditConflict(_)) => {
                ExitStatus::Diagnostics
            }
        }
    }
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

        assert_eq!(error.exit_status(), ExitStatus::InputOutput);
        assert_eq!(
            error.to_string(),
            "file updates completed with failures: files=3, changed=2, updated=1, unchanged=1, failed=1"
        );
    }
}
