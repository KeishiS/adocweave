//! Guarded in-place writes and user-visible file differences.

use std::io;
use std::path::PathBuf;

use crate::arguments::ColorChoice;

pub(crate) struct PendingWrite {
    pub(crate) path: PathBuf,
    pub(crate) replacement: Vec<u8>,
    pub(crate) capability: adocweave_project::ProjectWriteCapability,
}

impl PendingWrite {
    fn contents_match(&self) -> Result<bool, String> {
        self.capability
            .contents_match()
            .map_err(|error| error.to_string())
    }

    fn replace_after_recheck(self) -> Result<bool, String> {
        self.capability
            .replace_after_recheck(&self.replacement)
            .map_err(|error| error.to_string())
    }
}

pub(crate) struct WriteFailure {
    pub(crate) path: PathBuf,
    pub(crate) message: String,
}

#[derive(Default)]
pub(crate) struct WriteOutcome {
    pub(crate) updated: usize,
    pub(crate) failures: Vec<WriteFailure>,
}

pub(crate) fn apply_file_writes(writes: Vec<PendingWrite>) -> WriteOutcome {
    let mut outcome = WriteOutcome::default();
    for write in writes {
        let path = write.path.clone();
        let result = write.contents_match().and_then(|unchanged| {
            if !unchanged {
                return Err("input changed after it was read".to_owned());
            }
            write.replace_after_recheck().and_then(|replaced| {
                replaced
                    .then_some(())
                    .ok_or_else(|| "input changed after it was read".to_owned())
            })
        });
        match result {
            Ok(()) => outcome.updated += 1,
            Err(message) => outcome.failures.push(WriteFailure { path, message }),
        }
    }
    outcome
}

pub(crate) fn colorize_lines(output: &str, choice: ColorChoice) -> String {
    use std::io::IsTerminal as _;

    let enabled = match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => io::stdout().is_terminal(),
    };
    if !enabled {
        return output.to_owned();
    }
    let mut colored = String::new();
    for line in output.split_inclusive('\n') {
        let color = if line.starts_with('+') || line.contains(": hint[") {
            Some("\u{1b}[32m")
        } else if line.starts_with('-') || line.contains(": error[") {
            Some("\u{1b}[31m")
        } else if line.contains(": warning[") {
            Some("\u{1b}[33m")
        } else if line.contains(": information[") {
            Some("\u{1b}[36m")
        } else {
            None
        };
        if let Some(color) = color {
            colored.push_str(color);
            colored.push_str(line.trim_end_matches('\n'));
            colored.push_str("\u{1b}[0m");
            if line.ends_with('\n') {
                colored.push('\n');
            }
        } else {
            colored.push_str(line);
        }
    }
    colored
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use adocweave_core::NeverCancel;
    use adocweave_project::{
        ProjectAuthority, ProjectConfigOverrides, ProjectConfigSelection, ProjectLimits,
        ProjectRequest, ProjectTarget, process,
    };

    use super::*;

    fn temporary_directory(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("adocweave-{name}-{unique}"));
        fs::create_dir(&path).expect("temporary directory");
        path
    }

    fn capability(
        root: &std::path::Path,
        file_name: &str,
    ) -> adocweave_project::ProjectWriteCapability {
        let authority =
            ProjectAuthority::open(root.to_owned(), [root.to_owned()]).expect("project authority");
        let result = process(
            ProjectRequest {
                targets: vec![ProjectTarget::Path(PathBuf::from(file_name))],
                sources: Vec::new(),
                config: ProjectConfigSelection::Disabled,
                overrides: ProjectConfigOverrides::default(),
                apply_safe_fixes: false,
                resource_selection: Default::default(),
                authority,
                limits: ProjectLimits::default(),
            },
            &NeverCancel,
        )
        .expect("project processing");
        result
            .targets
            .into_iter()
            .next()
            .expect("one target")
            .write
            .expect("file target has write authority")
    }

    #[test]
    fn concurrent_content_change_is_never_replaced() {
        let root = temporary_directory("concurrent-write");
        let path = root.join("document.adoc");
        fs::write(&path, "original\n").expect("original");
        let capability = capability(&root, "document.adoc");
        let pending = PendingWrite {
            path: path.clone(),
            replacement: b"formatted\n".to_vec(),
            capability,
        };
        fs::write(&path, "concurrent\n").expect("concurrent update");

        let outcome = apply_file_writes(vec![pending]);
        assert_eq!(outcome.updated, 0);
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].path, path);
        assert_eq!(fs::read_to_string(&path).unwrap(), "concurrent\n");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn one_failed_replacement_does_not_roll_back_an_updated_file() {
        let root = temporary_directory("preflight-write");
        let first = root.join("first.adoc");
        let second = root.join("second.adoc");
        let third = root.join("third.adoc");
        fs::write(&first, "first\n").expect("first");
        fs::write(&second, "second\n").expect("second");
        fs::write(&third, "third\n").expect("third");
        let first_capability = capability(&root, "first.adoc");
        let second_capability = capability(&root, "second.adoc");
        let third_capability = capability(&root, "third.adoc");
        fs::write(&second, "concurrent\n").expect("concurrent update");
        let writes = vec![
            PendingWrite {
                path: first.clone(),
                replacement: b"changed\n".to_vec(),
                capability: first_capability,
            },
            PendingWrite {
                path: second.clone(),
                replacement: b"changed\n".to_vec(),
                capability: second_capability,
            },
            PendingWrite {
                path: third.clone(),
                replacement: b"changed\n".to_vec(),
                capability: third_capability,
            },
        ];

        let outcome = apply_file_writes(writes);
        assert_eq!(outcome.updated, 2);
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(fs::read_to_string(&first).unwrap(), "changed\n");
        assert_eq!(fs::read_to_string(&second).unwrap(), "concurrent\n");
        assert_eq!(fs::read_to_string(&third).unwrap(), "changed\n");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn retained_write_cannot_be_redirected_by_root_replacement() {
        use std::os::unix::fs::symlink;

        let parent = temporary_directory("retained-write-parent");
        let root = parent.join("workspace");
        let outside = parent.join("outside");
        fs::create_dir(&root).expect("workspace");
        fs::create_dir(&outside).expect("outside directory");
        let path = root.join("document.adoc");
        let outside_path = outside.join("document.adoc");
        fs::write(&path, "original\n").expect("trusted input");
        fs::write(&outside_path, "outside\n").expect("outside input");
        let capability = capability(&root, "document.adoc");
        let displaced = parent.join("retained-workspace");
        fs::rename(&root, &displaced).expect("displace workspace");
        symlink(&outside, &root).expect("replacement symlink");

        let outcome = apply_file_writes(vec![PendingWrite {
            path: path.clone(),
            replacement: b"formatted\n".to_vec(),
            capability,
        }]);
        assert_eq!(outcome.updated, 1);
        assert!(outcome.failures.is_empty());

        assert_eq!(
            fs::read(displaced.join("document.adoc")).expect("retained result"),
            b"formatted\n"
        );
        assert_eq!(
            fs::read(&outside_path).expect("outside result"),
            b"outside\n"
        );
        fs::remove_file(&root).expect("remove replacement symlink");
        fs::rename(displaced, &root).expect("restore workspace");
        fs::remove_dir_all(parent).expect("cleanup");
    }
}
