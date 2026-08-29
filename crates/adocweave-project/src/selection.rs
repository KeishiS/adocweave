use std::collections::{BTreeMap, BTreeSet};
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

pub(crate) fn select_targets(
    project_root: &Path,
    selectors: &[ProjectTarget],
    authority: &mut LocalFilesystemPolicy,
    limits: ProjectLimits,
    job: &IncludeFilesystemJob,
    warnings: &mut Vec<ProjectWarning>,
) -> Result<Vec<PathBuf>, ProjectError> {
    let mut selected = BTreeSet::new();
    let mut scans = BTreeMap::<PathBuf, Vec<PathBuf>>::new();
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
                let paths = scan_once(&directory, authority, limits, job, warnings, &mut scans)?;
                selected.extend(paths.iter().cloned());
            }
            ProjectTarget::Glob(authored) => {
                let pattern = GlobPattern::parse(authored)?;
                let scan_root = absolute_lexical(project_root, &pattern.scan_root)
                    .map_err(ProjectError::Authority)?;
                if !scan_root.starts_with(project_root) {
                    return Err(ProjectError::TargetSelection(
                        TargetSelectionError::InvalidGlob {
                            pattern: authored.clone(),
                        },
                    ));
                }
                let paths = scan_once(&scan_root, authority, limits, job, warnings, &mut scans)?;
                selected.extend(
                    paths
                        .iter()
                        .filter(|path| {
                            path.strip_prefix(project_root)
                                .ok()
                                .is_some_and(|relative| {
                                    pattern.matches(&logical_path(Path::new(""), relative))
                                })
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
    warnings: &mut Vec<ProjectWarning>,
    scans: &'scan mut BTreeMap<PathBuf, Vec<PathBuf>>,
) -> Result<&'scan [PathBuf], ProjectError> {
    if !scans.contains_key(directory) {
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
        let excludes = WorkspaceScanSettings::default();
        let (paths, complete) = transaction
            .discover_adoc_paths_within_budget(|_, relative| excludes.excludes(relative))
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
        scans.insert(directory.to_owned(), paths);
    }
    Ok(scans
        .get(directory)
        .expect("a completed scan is retained for the request"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum GlobToken {
    Literal(char),
    AnyOne,
    AnySegment,
    AnyRecursive,
    RecursiveDirectories,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

#[derive(Clone, Debug)]
struct GlobPattern {
    scan_root: PathBuf,
    tokens: Vec<GlobToken>,
}

impl GlobPattern {
    fn parse(authored: &str) -> Result<Self, ProjectError> {
        if authored.is_empty() || Path::new(authored).is_absolute() || authored.contains('\\') {
            return Err(invalid_glob(authored));
        }
        let mut tokens = Vec::new();
        let chars = authored.chars().collect::<Vec<_>>();
        let mut index = 0;
        while index < chars.len() {
            match chars[index] {
                '?' => tokens.push(GlobToken::AnyOne),
                '*' if chars.get(index + 1) == Some(&'*') => {
                    index += 1;
                    if chars.get(index + 1) == Some(&'/') {
                        index += 1;
                        tokens.push(GlobToken::RecursiveDirectories);
                    } else {
                        tokens.push(GlobToken::AnyRecursive);
                    }
                }
                '*' => tokens.push(GlobToken::AnySegment),
                '[' => {
                    let (token, end) = parse_class(authored, &chars, index)?;
                    tokens.push(token);
                    index = end;
                }
                literal => tokens.push(GlobToken::Literal(literal)),
            }
            index += 1;
        }
        let wildcard = authored
            .char_indices()
            .find_map(|(index, value)| matches!(value, '*' | '?' | '[').then_some(index))
            .unwrap_or(authored.len());
        let literal = &authored[..wildcard];
        let scan_root = literal.rfind('/').map_or_else(PathBuf::new, |separator| {
            PathBuf::from(&literal[..separator])
        });
        Ok(Self { scan_root, tokens })
    }

    fn matches(&self, path: &str) -> bool {
        let path = path.chars().collect::<Vec<_>>();
        let mut memo = BTreeMap::new();
        match_tokens(&self.tokens, &path, 0, 0, &mut memo)
    }
}

fn parse_class(
    authored: &str,
    chars: &[char],
    start: usize,
) -> Result<(GlobToken, usize), ProjectError> {
    let mut index = start + 1;
    let negated = chars
        .get(index)
        .is_some_and(|value| matches!(value, '!' | '^'));
    if negated {
        index += 1;
    }
    let mut ranges = Vec::new();
    while let Some(&value) = chars.get(index) {
        if value == ']' {
            if ranges.is_empty() {
                return Err(invalid_glob(authored));
            }
            return Ok((GlobToken::Class { negated, ranges }, index));
        }
        if let (Some('-'), Some(&end)) = (chars.get(index + 1), chars.get(index + 2))
            && end != ']'
        {
            if value > end {
                return Err(invalid_glob(authored));
            }
            ranges.push((value, end));
            index += 3;
        } else {
            ranges.push((value, value));
            index += 1;
        }
    }
    Err(invalid_glob(authored))
}

fn invalid_glob(authored: &str) -> ProjectError {
    ProjectError::TargetSelection(TargetSelectionError::InvalidGlob {
        pattern: authored.to_owned(),
    })
}

fn match_tokens(
    pattern: &[GlobToken],
    path: &[char],
    pattern_index: usize,
    path_index: usize,
    memo: &mut BTreeMap<(usize, usize), bool>,
) -> bool {
    if let Some(result) = memo.get(&(pattern_index, path_index)) {
        return *result;
    }
    let result = match pattern.get(pattern_index) {
        None => path_index == path.len(),
        Some(GlobToken::Literal(expected)) => {
            path.get(path_index) == Some(expected)
                && match_tokens(pattern, path, pattern_index + 1, path_index + 1, memo)
        }
        Some(GlobToken::AnyOne) => {
            path.get(path_index).is_some_and(|value| *value != '/')
                && match_tokens(pattern, path, pattern_index + 1, path_index + 1, memo)
        }
        Some(GlobToken::AnySegment) => {
            match_tokens(pattern, path, pattern_index + 1, path_index, memo)
                || path.get(path_index).is_some_and(|value| *value != '/')
                    && match_tokens(pattern, path, pattern_index, path_index + 1, memo)
        }
        Some(GlobToken::AnyRecursive) => {
            match_tokens(pattern, path, pattern_index + 1, path_index, memo)
                || path_index < path.len()
                    && match_tokens(pattern, path, pattern_index, path_index + 1, memo)
        }
        Some(GlobToken::RecursiveDirectories) => {
            match_tokens(pattern, path, pattern_index + 1, path_index, memo)
                || path.get(path_index).is_some_and(|value| *value != '/')
                    && match_tokens(pattern, path, pattern_index, path_index + 1, memo)
                || path.get(path_index) == Some(&'/')
                    && match_tokens(pattern, path, pattern_index, path_index + 1, memo)
        }
        Some(GlobToken::Class { negated, ranges }) => {
            path.get(path_index)
                .filter(|value| **value != '/')
                .is_some_and(|value| {
                    let contained = ranges
                        .iter()
                        .any(|(start, end)| start <= value && value <= end);
                    contained != *negated
                })
                && match_tokens(pattern, path, pattern_index + 1, path_index + 1, memo)
        }
    };
    memo.insert((pattern_index, path_index), result);
    result
}

#[cfg(test)]
mod tests {
    use super::GlobPattern;

    #[test]
    fn glob_matches_segments_recursive_directories_and_classes() {
        let recursive = GlobPattern::parse("docs/**/*.adoc").expect("valid glob");
        assert!(recursive.matches("docs/guide.adoc"));
        assert!(recursive.matches("docs/user/guide.adoc"));
        assert!(!recursive.matches("guide.adoc"));

        let class = GlobPattern::parse("docs/[a-c]?.adoc").expect("valid class");
        assert!(class.matches("docs/b1.adoc"));
        assert!(!class.matches("docs/d1.adoc"));
    }
}
