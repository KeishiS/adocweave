use std::path::{Path, PathBuf};

use adocweave_core::output::formatter::{FormatConfig, FormatError, NewlineStyle, format_analysis};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Options {
    pub(crate) check: bool,
    pub(crate) write: bool,
    pub(crate) diff: bool,
    pub(crate) summary: bool,
}

impl Options {
    pub(crate) const fn supports_multiple_inputs(self) -> bool {
        self.check || self.write || self.diff
    }

    fn uses_stable_line_endings(self) -> bool {
        self.check || self.write || self.diff
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    InvalidUtf8 { valid_up_to: usize },
    Position(adocweave_core::text::PositionError),
}

pub(crate) struct BatchOutcome {
    pub(crate) output: String,
    pub(crate) pending_writes: Vec<WriteRequest>,
    pub(crate) files: usize,
    pub(crate) changed: usize,
    pub(crate) formatting_required: bool,
}

pub(crate) struct WriteRequest {
    pub(crate) path: PathBuf,
    pub(crate) replacement: Vec<u8>,
}

impl BatchOutcome {
    pub(crate) fn summary(&self) -> String {
        format!(
            "adocweave format: files={}, changed={}",
            self.files, self.changed
        )
    }
}

pub(crate) fn project_format_config(
    options: Options,
    input: &str,
    project: &adocweave_project::ProjectConfig,
) -> FormatConfig {
    let mut format = *project.format();
    if options.uses_stable_line_endings() && !project.format_newline_explicit() {
        let bytes = input.as_bytes();
        let crlf = bytes.windows(2).filter(|window| *window == b"\r\n").count();
        let lf = bytes.iter().filter(|byte| **byte == b'\n').count();
        format.newline = if crlf > 0 && crlf.saturating_mul(2) >= lf {
            NewlineStyle::CrLf
        } else {
            NewlineStyle::Lf
        };
    }
    if options.uses_stable_line_endings() && !project.format_final_newline_explicit() {
        format.final_newline = input.ends_with('\n');
    }
    format
}

pub(crate) fn process_analysis(
    analysis: &adocweave_core::Analysis,
    format_config: &FormatConfig,
) -> Result<String, Error> {
    match format_analysis(analysis, format_config, &adocweave_core::NeverCancel) {
        Ok(output) => Ok(output.formatted),
        Err(FormatError::Position(error)) => Err(Error::Position(error)),
        Err(FormatError::Cancelled) => unreachable!("NeverCancel cannot cancel formatting"),
    }
}

pub(crate) struct BatchWorkflow {
    options: Options,
    files: usize,
    changed: usize,
    output: String,
    pending_writes: Vec<WriteRequest>,
}

impl BatchWorkflow {
    pub(crate) fn new(options: Options, files: usize) -> Self {
        Self {
            options,
            files,
            changed: 0,
            output: String::new(),
            pending_writes: Vec::new(),
        }
    }

    pub(crate) fn record(
        &mut self,
        path: PathBuf,
        original: Vec<u8>,
        formatted: Vec<u8>,
    ) -> Result<(), Error> {
        if original == formatted {
            return Ok(());
        }
        self.changed += 1;
        if self.options.diff {
            self.output.push_str(&unified_diff(
                &path,
                decode_input(&original)?,
                decode_input(&formatted)?,
            ));
        }
        if self.options.write {
            self.pending_writes.push(WriteRequest {
                path,
                replacement: formatted,
            });
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> BatchOutcome {
        BatchOutcome {
            output: self.output,
            pending_writes: self.pending_writes,
            files: self.files,
            changed: self.changed,
            formatting_required: self.options.check && self.changed > 0,
        }
    }
}

fn decode_input(input: &[u8]) -> Result<&str, Error> {
    std::str::from_utf8(input).map_err(|error| Error::InvalidUtf8 {
        valid_up_to: error.valid_up_to(),
    })
}

pub(crate) fn unified_diff(path: &Path, original: &str, formatted: &str) -> String {
    similar::TextDiff::from_lines(original, formatted)
        .unified_diff()
        .header(
            &format!("a/{}", path.display()),
            &format!("b/{}", path.display()),
        )
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{BatchWorkflow, Options};

    #[test]
    fn batch_collects_diff_and_write_decisions_without_touching_files() {
        let options = Options {
            write: true,
            diff: true,
            ..Options::default()
        };
        let mut workflow = BatchWorkflow::new(options, 2);
        workflow
            .record("a.adoc".into(), b"a  \n".to_vec(), b"a\n".to_vec())
            .expect("changed document");
        workflow
            .record("b.adoc".into(), b"b\n".to_vec(), b"b\n".to_vec())
            .expect("unchanged document");

        let outcome = workflow.finish();
        assert_eq!(outcome.files, 2);
        assert_eq!(outcome.changed, 1);
        assert_eq!(outcome.pending_writes.len(), 1);
        assert!(outcome.output.contains("--- a/a.adoc"));
        assert!(!outcome.formatting_required);
    }

    #[test]
    fn check_failure_is_decided_after_the_complete_batch() {
        let mut workflow = BatchWorkflow::new(
            Options {
                check: true,
                ..Options::default()
            },
            1,
        );
        workflow
            .record(
                "manual.adoc".into(),
                b"text  \n".to_vec(),
                b"text\n".to_vec(),
            )
            .expect("changed document");

        let outcome = workflow.finish();
        assert!(outcome.formatting_required);
        assert!(outcome.pending_writes.is_empty());
    }
}
