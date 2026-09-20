//! What this terminal can do, decided in one place.
//!
//! The core backend lays a document out and says what each piece of text is;
//! it never asks where the output goes. Everything that depends on the
//! environment is decided here: how wide the terminal is, and whether the
//! reader wants color.

use std::io::{self, IsTerminal as _};

use crate::arguments::ColorChoice;

/// The width used when nothing says otherwise. It is the width a terminal has
/// had by default for as long as terminals have had one.
const DEFAULT_WIDTH: u16 = 80;

/// The narrowest and widest output this accepts. A single column holds no
/// text, and a width beyond a few hundred columns is a typing mistake rather
/// than a terminal.
pub(crate) const MIN_WIDTH: u16 = 20;
pub(crate) const MAX_WIDTH: u16 = 1000;

/// How many columns the output may use.
///
/// `--width` is what the reader asked for and always wins. Otherwise the
/// environment is asked: `COLUMNS`, which a shell may export, and then the
/// terminal itself. A pipe has no width, so the default stands.
pub(crate) fn width(requested: Option<u16>) -> u16 {
    requested
        .or_else(environment_width)
        .or_else(terminal_width)
        .unwrap_or(DEFAULT_WIDTH)
        .clamp(MIN_WIDTH, MAX_WIDTH)
}

fn environment_width() -> Option<u16> {
    std::env::var("COLUMNS")
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|columns| *columns > 0)
}

fn terminal_width() -> Option<u16> {
    io::stdout()
        .is_terminal()
        .then(terminal_size::terminal_size)
        .flatten()
        .map(|(terminal_size::Width(columns), _)| columns)
}

/// Whether the output carries color.
///
/// `always` and `never` are the reader's decision and are followed. `auto`
/// means: color a terminal, leave a pipe or a file alone, and respect
/// `NO_COLOR`, which a reader sets once for every program they run.
pub(crate) fn color_enabled(choice: ColorChoice) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => io::stdout().is_terminal() && !no_color_requested(),
    }
}

/// `NO_COLOR` asks for no color when it is set to anything but an empty value.
fn no_color_requested() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_width_is_used_as_it_stands() {
        assert_eq!(width(Some(42)), 42);
    }

    #[test]
    fn a_width_outside_the_accepted_range_is_brought_back_into_it() {
        assert_eq!(width(Some(1)), MIN_WIDTH);
        assert_eq!(width(Some(u16::MAX)), MAX_WIDTH);
    }

    #[test]
    fn color_follows_an_explicit_choice_whatever_the_output_is() {
        assert!(color_enabled(ColorChoice::Always));
        assert!(!color_enabled(ColorChoice::Never));
    }

    /// A test runs with its output captured, which is not a terminal.
    #[test]
    fn automatic_color_stays_off_when_the_output_is_not_a_terminal() {
        assert!(!color_enabled(ColorChoice::Auto));
    }
}
