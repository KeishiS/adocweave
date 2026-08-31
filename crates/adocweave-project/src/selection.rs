use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use adocweave_core::CancellationCheck;

use crate::filesystem::{FilesystemAuthority, FilesystemError};
use crate::{
    MAX_DISTINCT_GLOB_SELECTORS, MAX_TOTAL_GLOB_PATTERN_BYTES, ProjectError, ProjectLimits,
    ProjectTarget, ProjectWarning, TargetSelectionError, project_authority_error,
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum NormalizedSelector {
    Path(PathBuf),
    Directory(PathBuf),
    Glob { pattern: String, scan_root: PathBuf },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ScanMode {
    Directory,
    Glob,
}

struct ScanState<'request> {
    authority: &'request FilesystemAuthority,
    limits: ProjectLimits,
    warnings: &'request mut Vec<ProjectWarning>,
    scans: BTreeMap<(ScanMode, PathBuf), Vec<PathBuf>>,
    directory_operations: &'request mut u64,
    directory_entries: &'request mut u64,
    cancellation: &'request dyn CancellationCheck,
}

pub(crate) fn normalize_selectors(
    project_root: &Path,
    authority: &FilesystemAuthority,
    selectors: &[ProjectTarget],
) -> Result<Vec<NormalizedSelector>, ProjectError> {
    let distinct_globs = selectors
        .iter()
        .filter_map(|selector| match selector {
            ProjectTarget::Glob(pattern) => Some(pattern.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if distinct_globs.len() > MAX_DISTINCT_GLOB_SELECTORS {
        return Err(ProjectError::TargetSelection(
            TargetSelectionError::TooManyGlobs {
                limit: MAX_DISTINCT_GLOB_SELECTORS,
            },
        ));
    }
    let pattern_bytes = distinct_globs
        .iter()
        .try_fold(0_usize, |total, pattern| total.checked_add(pattern.len()))
        .unwrap_or(usize::MAX);
    if pattern_bytes > MAX_TOTAL_GLOB_PATTERN_BYTES {
        return Err(ProjectError::TargetSelection(
            TargetSelectionError::GlobPatternBytes {
                limit: MAX_TOTAL_GLOB_PATTERN_BYTES,
            },
        ));
    }
    let mut seen_globs = BTreeSet::new();
    let mut authored = selectors
        .iter()
        .filter(|selector| match selector {
            ProjectTarget::Glob(pattern) => seen_globs.insert(pattern.as_str()),
            _ => true,
        })
        .collect::<Vec<_>>();
    authored.sort_by_key(|selector| selector_sort_key(selector));
    let mut normalized = authored
        .into_iter()
        .map(|selector| normalize_selector(project_root, authority, selector))
        .collect::<Result<Vec<_>, _>>()?;
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn selector_sort_key(selector: &ProjectTarget) -> (u8, String) {
    match selector {
        ProjectTarget::Source(source_id) => (0, source_id.as_str().to_owned()),
        ProjectTarget::Path(path) => (1, format!("{path:?}")),
        ProjectTarget::PathNoSymlinks(path) => (1, format!("{path:?}")),
        ProjectTarget::Directory(path) => (2, format!("{path:?}")),
        ProjectTarget::Glob(pattern) => (3, pattern.clone()),
    }
}

fn normalize_selector(
    project_root: &Path,
    authority: &FilesystemAuthority,
    selector: &ProjectTarget,
) -> Result<NormalizedSelector, ProjectError> {
    match selector {
        ProjectTarget::Source(_) => {
            unreachable!("source targets are resolved before selector normalization")
        }
        ProjectTarget::Path(path) => absolute_lexical(project_root, path)
            .and_then(|path| authority.normalize_path(&path))
            .map(NormalizedSelector::Path)
            .map_err(project_authority_error),
        ProjectTarget::PathNoSymlinks(path) => absolute_lexical(project_root, path)
            .and_then(|path| authority.normalize_path(&path))
            .map(NormalizedSelector::Path)
            .map_err(project_authority_error),
        ProjectTarget::Directory(path) => absolute_lexical(project_root, path)
            .and_then(|path| authority.normalize_path(&path))
            .map(NormalizedSelector::Directory)
            .map_err(project_authority_error),
        ProjectTarget::Glob(authored) => {
            glob::Pattern::new(authored).map_err(|_| invalid_glob(authored))?;
            let absolute_pattern = absolute_lexical(project_root, Path::new(authored))
                .and_then(|path| authority.normalize_path(&path))
                .map_err(project_authority_error)?;
            let pattern = absolute_pattern
                .to_str()
                .ok_or_else(|| invalid_glob(authored))?
                .to_owned();
            glob::Pattern::new(&pattern).map_err(|_| invalid_glob(authored))?;
            let scan_root = absolute_lexical(project_root, &glob_scan_root(authored))
                .and_then(|path| authority.normalize_path(&path))
                .map_err(project_authority_error)?;
            Ok(NormalizedSelector::Glob { pattern, scan_root })
        }
    }
}

/// Resolves `path` against `root` without touching the filesystem.
///
/// A relative path is joined to `root`. `.` is dropped and `..` removes the preceding component.
/// A `..` that would leave the filesystem root is rejected instead of being clamped, so a caller
/// never receives a wider directory than the one it asked for.
pub(crate) fn absolute_lexical(root: &Path, path: &Path) -> Result<PathBuf, FilesystemError> {
    let input = if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in input.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(FilesystemError::OutsideRoot(input));
                }
            }
        }
    }
    if !normalized.is_absolute() {
        return Err(FilesystemError::PathNotAbsolute(normalized));
    }
    Ok(normalized)
}

pub(crate) fn identity_path(root: &Path, path: &Path) -> String {
    let selected = path.strip_prefix(root).unwrap_or(path);
    selected
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(encode_os_string(value)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_os_string(value: &std::ffi::OsStr) -> String {
    if let Some(value) = value.to_str() {
        let mut encoded = String::new();
        for character in value.chars() {
            if character == '%' || character.is_control() {
                for byte in character.to_string().bytes() {
                    let _ = write!(encoded, "%{byte:02X}");
                }
            } else {
                encoded.push(character);
            }
        }
        return encoded;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let mut encoded = String::new();
        for byte in value.as_bytes() {
            let _ = write!(encoded, "%{byte:02X}");
        }
        encoded
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        let mut encoded = String::new();
        for unit in value.encode_wide() {
            let _ = write!(encoded, "%u{unit:04X}");
        }
        encoded
    }
    #[cfg(not(any(unix, windows)))]
    {
        value.to_string_lossy().replace('%', "%25")
    }
}

pub(crate) fn select_targets(
    selectors: &[NormalizedSelector],
    authority: &FilesystemAuthority,
    limits: ProjectLimits,
    warnings: &mut Vec<ProjectWarning>,
    directory_operations: &mut u64,
    directory_entries: &mut u64,
    cancellation: &dyn CancellationCheck,
) -> Result<Vec<PathBuf>, ProjectError> {
    let mut selected = BTreeSet::new();
    let mut scans = ScanState {
        authority,
        limits,
        warnings,
        scans: BTreeMap::new(),
        directory_operations,
        directory_entries,
        cancellation,
    };
    for selector in selectors {
        match selector {
            NormalizedSelector::Path(path) => {
                require_authority(scans.authority, path)?;
                selected.insert(path.clone());
            }
            NormalizedSelector::Directory(directory) => {
                let paths = scans.scan_once(directory, ScanMode::Directory)?;
                selected.extend(paths.iter().cloned());
            }
            NormalizedSelector::Glob { pattern, scan_root } => {
                let pattern = glob::Pattern::new(pattern)
                    .expect("normalized selectors retain a valid glob pattern");
                let paths = scans.scan_once(scan_root, ScanMode::Glob)?;
                selected.extend(
                    paths
                        .iter()
                        .filter(|path| pattern.matches_path(path))
                        .cloned(),
                );
            }
        }
    }
    Ok(selected.into_iter().collect())
}

fn require_authority(authority: &FilesystemAuthority, path: &Path) -> Result<(), ProjectError> {
    authority
        .authority_for_path(path)
        .map(|_| ())
        .ok_or_else(|| project_authority_error(FilesystemError::OutsideRoot(path.to_owned())))
}

impl ScanState<'_> {
    fn scan_once(
        &mut self,
        directory: &Path,
        mode: ScanMode,
    ) -> Result<Vec<PathBuf>, ProjectError> {
        let key = (mode, directory.to_owned());
        if let Some(paths) = self.scans.get(&key) {
            return Ok(paths.clone());
        }
        let result = self
            .authority
            .scan_adoc(
                directory,
                self.limits.max_directory_entries,
                self.cancellation,
            )
            .map_err(|error| {
                if self.cancellation.is_cancelled() {
                    ProjectError::Cancelled
                } else {
                    project_authority_error(error)
                }
            })?;
        *self.directory_operations = self.directory_operations.saturating_add(result.directories);
        *self.directory_entries = self.directory_entries.saturating_add(result.entries);
        if !result.complete
            && !self
                .warnings
                .iter()
                .any(|warning| matches!(warning, ProjectWarning::ScanTruncated { .. }))
        {
            self.warnings.push(ProjectWarning::ScanTruncated {
                limit: self.limits.max_directory_entries,
            });
        }
        self.scans.insert(key, result.paths.clone());
        Ok(result.paths)
    }
}

fn invalid_glob(authored: &str) -> ProjectError {
    ProjectError::TargetSelection(TargetSelectionError::InvalidGlob {
        pattern: authored.to_owned(),
    })
}

fn glob_scan_root(pattern: &str) -> PathBuf {
    let mut root = PathBuf::new();
    for component in Path::new(pattern).components() {
        let authored = component.as_os_str().to_string_lossy();
        if authored
            .chars()
            .any(|value| matches!(value, '*' | '?' | '['))
        {
            return if root.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                root
            };
        }
        root.push(component.as_os_str());
    }
    root.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_owned)
}

#[cfg(test)]
mod tests {
    use glob::Pattern;

    #[test]
    fn glob_matches_segments_recursive_directories_and_classes() {
        let recursive = Pattern::new("docs/**/*.adoc").expect("valid glob");
        assert!(recursive.matches_path(std::path::Path::new("docs/guide.adoc")));
        assert!(recursive.matches_path(std::path::Path::new("docs/user/guide.adoc")));
        assert!(!recursive.matches_path(std::path::Path::new("guide.adoc")));

        let class = Pattern::new("docs/[a-c]?.adoc").expect("valid class");
        assert!(class.matches_path(std::path::Path::new("docs/b1.adoc")));
        assert!(!class.matches_path(std::path::Path::new("docs/d1.adoc")));
    }
}
