//! Process exit codes owned by the command-line interface.

use std::process::ExitCode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum CliExitCode {
    Success = 0,
    Diagnostics = 1,
    Usage = 2,
    InputOutput = 3,
    LimitExceeded = 4,
}

impl CliExitCode {
    pub(crate) const fn code(self) -> u8 {
        self as u8
    }

    pub(crate) fn from_clap(code: i32) -> Self {
        if code == 0 {
            Self::Success
        } else {
            Self::Usage
        }
    }
}

impl From<CliExitCode> for ExitCode {
    fn from(code: CliExitCode) -> Self {
        Self::from(code.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cli_exit_code_keeps_its_number() {
        assert_eq!(CliExitCode::Success.code(), 0);
        assert_eq!(CliExitCode::Diagnostics.code(), 1);
        assert_eq!(CliExitCode::Usage.code(), 2);
        assert_eq!(CliExitCode::InputOutput.code(), 3);
        assert_eq!(CliExitCode::LimitExceeded.code(), 4);
    }

    #[test]
    fn clap_success_and_failure_use_the_cli_categories() {
        assert_eq!(CliExitCode::from_clap(0), CliExitCode::Success);
        assert_eq!(CliExitCode::from_clap(2), CliExitCode::Usage);
    }
}
