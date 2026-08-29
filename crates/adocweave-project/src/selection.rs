use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use adocweave_config::WorkspaceScanSettings;
use adocweave_host::{
    DerivedFilesystemRoots, IncludeFilesystemJob, LocalFilesystemPolicy, ResourceError,
};

use crate::{ProjectError, ProjectLimits, ProjectTarget, ProjectWarning, TargetSelectionError};

pub(crate) fn absolute_lexical(root: &Path, path: &Path) -> Result<PathBuf, ResourceError> {
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
                    return Err(ResourceError::OutsideRoots(input));
                }
            }
        }
    }
    if !normalized.is_absolute() {
        return Err(ResourceError::PathNotAbsolute(normalized));
    }
    Ok(normalized)
}

pub(crate) fn logical_path(root: &Path, path: &Path) -> String {
    let selected = path.strip_prefix(root).unwrap_or(path);
    selected
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
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
    project_root: &Path,
    selectors: &[ProjectTarget],
    authority: &mut LocalFilesystemPolicy,
    limits: ProjectLimits,
    job: &IncludeFilesystemJob,
    scan_settings: &BTreeMap<PathBuf, WorkspaceScanSettings>,
    warnings: &mut Vec<ProjectWarning>,
) -> Result<Vec<PathBuf>, ProjectError> {
    let mut selected = BTreeSet::new();
    let mut scans = BTreeMap::<(PathBuf, Vec<String>), Vec<PathBuf>>::new();
    for selector in selectors {
        match selector {
            ProjectTarget::Path(path) => {
                let path = absolute_lexical(project_root, path).map_err(ProjectError::Authority)?;
                require_authority(authority, &path)?;
                selected.insert(path);
            }
            ProjectTarget::Directory(directory) => {
                let directory =
                    absolute_lexical(project_root, directory).map_err(ProjectError::Authority)?;
                let paths = scan_once(
                    &directory, authority, limits, job, None, warnings, &mut scans,
                )?;
                selected.extend(paths.iter().cloned());
            }
            ProjectTarget::Workspace(directory) => {
                let directory =
                    absolute_lexical(project_root, directory).map_err(ProjectError::Authority)?;
                let paths = scan_once(
                    &directory,
                    authority,
                    limits,
                    job,
                    scan_settings.get(&directory),
                    warnings,
                    &mut scans,
                )?;
                selected.extend(paths.iter().cloned());
            }
            ProjectTarget::Glob(authored) => {
                let pattern = glob::Pattern::new(authored).map_err(|_| invalid_glob(authored))?;
                let scan_root = absolute_lexical(project_root, &glob_scan_root(authored))
                    .map_err(ProjectError::Authority)?;
                let absolute = Path::new(authored).is_absolute();
                let paths = scan_once(
                    &scan_root, authority, limits, job, None, warnings, &mut scans,
                )?;
                selected.extend(
                    paths
                        .iter()
                        .filter(|path| {
                            if absolute {
                                pattern.matches_path(path)
                            } else {
                                path.strip_prefix(project_root)
                                    .is_ok_and(|relative| pattern.matches_path(relative))
                            }
                        })
                        .cloned(),
                );
            }
        }
    }
    Ok(selected.into_iter().collect())
}

fn require_authority(authority: &LocalFilesystemPolicy, path: &Path) -> Result<(), ProjectError> {
    authority
        .policy_for_path(path)
        .map(|_| ())
        .ok_or_else(|| ProjectError::Authority(ResourceError::OutsideRoots(path.to_owned())))
}

fn scan_once<'scan>(
    directory: &Path,
    authority: &mut LocalFilesystemPolicy,
    limits: ProjectLimits,
    job: &IncludeFilesystemJob,
    scan_settings: Option<&WorkspaceScanSettings>,
    warnings: &mut Vec<ProjectWarning>,
    scans: &'scan mut BTreeMap<(PathBuf, Vec<String>), Vec<PathBuf>>,
) -> Result<&'scan [PathBuf], ProjectError> {
    let patterns = scan_settings
        .into_iter()
        .flat_map(WorkspaceScanSettings::exclude_patterns)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let key = (directory.to_owned(), patterns);
    if !scans.contains_key(&key) {
        let anchor = authority
            .policy_for_path(directory)
            .map(|policy| policy.root().to_owned())
            .ok_or_else(|| {
                ProjectError::Authority(ResourceError::OutsideRoots(directory.to_owned()))
            })?;
        let policy = if anchor == directory {
            authority.access_existing([anchor], limits.filesystem_reads)
        } else {
            authority.access_derived(
                &anchor,
                DerivedFilesystemRoots {
                    confined: vec![directory.to_owned()],
                    independent: Vec::new(),
                },
                limits.filesystem_reads,
            )
        }
        .map_err(ProjectError::Authority)?;
        let session = policy.session().map_err(ProjectError::Authority)?;
        let transaction = job
            .transaction(&session)
            .map_err(|error| ProjectError::Authority(ResourceError::from(error)))?;
        let (paths, complete) = transaction
            .discover_adoc_paths_within_budget(|_, relative| {
                scan_settings.is_some_and(|settings| settings.excludes(relative))
            })
            .map_err(|error| ProjectError::Authority(ResourceError::from(error)))?;
        if !complete
            && !warnings
                .iter()
                .any(|warning| matches!(warning, ProjectWarning::ScanTruncated { .. }))
        {
            warnings.push(ProjectWarning::ScanTruncated {
                limit: limits.max_directory_entries,
            });
        }
        scans.insert(key.clone(), paths);
    }
    Ok(scans
        .get(&key)
        .expect("a completed scan is retained for the request"))
}

fn invalid_glob(authored: &str) -> ProjectError {
    ProjectError::TargetSelection(TargetSelectionError::InvalidGlob {
        pattern: authored.to_owned(),
    })
}

pub(crate) fn scan_root_for_selector(
    project_root: &Path,
    selector: &ProjectTarget,
) -> Result<Option<PathBuf>, ProjectError> {
    match selector {
        ProjectTarget::Workspace(path) => absolute_lexical(project_root, path)
            .map(Some)
            .map_err(ProjectError::Authority),
        ProjectTarget::Path(_) | ProjectTarget::Directory(_) | ProjectTarget::Glob(_) => Ok(None),
    }
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
