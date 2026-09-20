//! Handing long output to the program the reader uses to page through it.
//!
//! A page that does not fit on the screen scrolls past before it can be read.
//! A pager is what a reader already uses for that, so the output is handed to
//! it rather than being paged here.

use std::io::{self, Write as _};
use std::process::{Child, Command, Stdio};

use crate::arguments::PagerChoice;

/// The pager used when the reader has named none. `-R` passes the style
/// through instead of showing it as text, and `-F` leaves output that fits on
/// one screen where it is.
const DEFAULT_PAGER: &str = "less";
const DEFAULT_PAGER_ARGUMENTS: &[&str] = &["-R", "-F"];

/// Whether the output is handed to a pager, and what the page then counts as.
///
/// A pager writes to the terminal even though this program writes to a pipe,
/// so the output is styled as it would be for a terminal.
pub(crate) fn wanted(choice: PagerChoice, lines: usize, stdout_is_terminal: bool) -> bool {
    match choice {
        PagerChoice::Never => false,
        PagerChoice::Always => true,
        // Switching to a pager for a page that already fits is a screen change
        // the reader did not ask for.
        PagerChoice::Auto => {
            stdout_is_terminal && crate::terminal::height().is_some_and(|height| lines > height)
        }
    }
}

/// Writes `output` through a pager, or reports that no pager could be started.
///
/// A missing pager is not a failure: the reader still gets the page, on
/// standard output, which is what they would have had anyway.
pub(crate) fn write(output: &str) -> Result<(), NoPager> {
    let mut child = spawn().ok_or(NoPager)?;
    if let Some(mut stdin) = child.stdin.take() {
        // A reader who quits the pager early closes its input. That is an
        // answer, not an error.
        match stdin.write_all(output.as_bytes()) {
            Ok(()) | Err(_) => drop(stdin),
        }
    }
    // The pager owns the screen until the reader leaves it. Its own exit
    // status says nothing about whether the document was rendered.
    let _ = child.wait();
    Ok(())
}

/// No pager could be started, so the caller writes the output itself.
#[derive(Debug)]
pub(crate) struct NoPager;

fn spawn() -> Option<Child> {
    for mut command in candidates(std::env::var_os("PAGER")) {
        if let Ok(child) = command.stdin(Stdio::piped()).spawn() {
            return Some(child);
        }
    }
    None
}

/// The pagers to try: the one the reader named in `PAGER`, or the usual one.
fn candidates(named: Option<std::ffi::OsString>) -> Vec<Command> {
    let Some(named) = named else {
        return vec![default_pager()];
    };
    let named = named.to_string_lossy().trim().to_owned();
    // `PAGER=` set to nothing is a way of asking for no pager at all.
    if named.is_empty() {
        return Vec::new();
    }
    // A named pager may carry arguments of its own, as `less -S` does. They
    // are separated by spaces, which is how a shell would read the value.
    let mut words = named.split_whitespace();
    let Some(program) = words.next() else {
        return vec![default_pager()];
    };
    let mut command = Command::new(program);
    command.args(words);
    vec![command]
}

fn default_pager() -> Command {
    let mut command = Command::new(DEFAULT_PAGER);
    command.args(DEFAULT_PAGER_ARGUMENTS);
    command
}

/// Writes `output` to standard output, treating a reader that stops early as
/// the answer it is.
pub(crate) fn write_to_standard_output(output: &str) -> io::Result<()> {
    match io::stdout().write_all(output.as_bytes()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_who_asked_for_no_pager_gets_none() {
        assert!(!wanted(PagerChoice::Never, 10_000, true));
    }

    #[test]
    fn a_reader_who_asked_for_a_pager_gets_one_whatever_the_output_is() {
        assert!(wanted(PagerChoice::Always, 1, false));
    }

    /// A test writes to a pipe, which no pager is started for.
    #[test]
    fn output_that_is_not_read_on_a_screen_needs_no_pager() {
        assert!(!wanted(PagerChoice::Auto, 10_000, false));
    }

    #[test]
    fn a_pager_named_as_nothing_asks_for_no_pager() {
        assert!(candidates(Some("".into())).is_empty());
        assert!(candidates(Some("   ".into())).is_empty());
    }

    #[test]
    fn a_named_pager_keeps_the_arguments_it_was_given() {
        let candidates = candidates(Some("less -S".into()));

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].get_program(), "less");
        let arguments: Vec<_> = candidates[0].get_args().collect();
        assert_eq!(arguments, ["-S"]);
    }

    #[test]
    fn the_usual_pager_passes_style_through_and_leaves_a_short_page_alone() {
        let candidates = candidates(None);

        assert_eq!(candidates[0].get_program(), DEFAULT_PAGER);
        let arguments: Vec<_> = candidates[0].get_args().collect();
        assert_eq!(arguments, DEFAULT_PAGER_ARGUMENTS);
    }
}
