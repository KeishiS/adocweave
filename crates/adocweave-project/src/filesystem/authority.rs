use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use adocweave::CancellationCheck;

use super::{FilesystemError, FilesystemRead, RootAuthority};

const MAX_ROOTS: usize = 128;

#[derive(Clone, Debug)]
pub(crate) struct FilesystemAuthority {
    roots: Vec<PathBuf>,
    authorities: Vec<RootAuthority>,
}

#[derive(Debug)]
pub(crate) struct ScanResult {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) entries: u64,
    pub(crate) directories: u64,
    pub(crate) complete: bool,
}

impl FilesystemAuthority {
    pub(crate) fn open(roots: impl IntoIterator<Item = PathBuf>) -> Result<Self, FilesystemError> {
        let mut unique = BTreeMap::<PathBuf, RootAuthority>::new();
        for root in roots {
            if !root.is_absolute() {
                return Err(FilesystemError::PathNotAbsolute(root));
            }
            let authority = RootAuthority::new(&root).map_err(|error| root_error(root, error))?;
            let root = authority.root().to_owned();
            if let Some(_existing) = unique.get_mut(&root) {
                #[cfg(not(target_os = "linux"))]
                _existing.merge_authored_roots(&authority);
                continue;
            }
            if unique.len() >= MAX_ROOTS {
                return Err(FilesystemError::LimitExceeded { limit: MAX_ROOTS });
            }
            unique.insert(root, authority);
        }
        if unique.is_empty() {
            return Err(FilesystemError::Unverifiable(
                "no local resource roots were configured".to_owned(),
            ));
        }
        Ok(Self::from_map(unique))
    }

    fn from_map(unique: BTreeMap<PathBuf, RootAuthority>) -> Self {
        let roots = unique.keys().cloned().collect();
        let authorities = unique.into_values().collect();
        Self { roots, authorities }
    }

    pub(crate) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub(crate) fn authority_for_path(&self, path: &Path) -> Option<&RootAuthority> {
        self.authorities
            .iter()
            .filter_map(|authority| {
                authority
                    .matching_prefix_depth(path)
                    .map(|depth| (depth, authority))
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, authority)| authority)
    }

    pub(crate) fn normalize_path(&self, path: &Path) -> Result<PathBuf, FilesystemError> {
        self.authority_for_path(path)
            .ok_or_else(|| FilesystemError::OutsideRoot(path.to_owned()))?
            .normalize_path(path)
    }

    pub(crate) fn authority_for_root(&self, root: &Path) -> Option<&RootAuthority> {
        self.authorities
            .iter()
            .find(|authority| authority.represents_root(root))
    }

    pub(crate) fn select(
        &self,
        roots: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, FilesystemError> {
        let mut unique = BTreeMap::new();
        for root in roots {
            let authority = self
                .authority_for_root(&root)
                .cloned()
                .ok_or_else(|| FilesystemError::OutsideRoot(root.clone()))?;
            unique.insert(root, authority);
        }
        if unique.is_empty() {
            return Err(FilesystemError::Unverifiable(
                "no local resource roots were selected".to_owned(),
            ));
        }
        Ok(Self::from_map(unique))
    }

    pub(crate) fn derive(
        &mut self,
        anchor: &Path,
        roots: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, FilesystemError> {
        let anchor_authority = self
            .authority_for_root(anchor)
            .cloned()
            .ok_or_else(|| FilesystemError::OutsideRoot(anchor.to_owned()))?;
        let mut pending = BTreeMap::new();
        for root in roots {
            let authority = if root == anchor {
                anchor_authority.clone()
            } else {
                anchor_authority.derive_confined_directory(&root)?
            };
            pending.insert(authority.root().to_owned(), authority);
        }
        let mut retained = self
            .authorities
            .iter()
            .cloned()
            .map(|authority| (authority.root().to_owned(), authority))
            .collect::<BTreeMap<_, _>>();
        for (root, authority) in &pending {
            if !retained.contains_key(root) && retained.len() >= MAX_ROOTS {
                return Err(FilesystemError::LimitExceeded { limit: MAX_ROOTS });
            }
            retained.insert(root.clone(), authority.clone());
        }
        *self = Self::from_map(retained);
        Ok(Self::from_map(pending))
    }

    pub(crate) fn read_utf8(
        &self,
        path: &Path,
        max_bytes: u64,
        no_symlinks: bool,
    ) -> FilesystemRead {
        self.authority_for_path(path).map_or_else(
            || FilesystemRead::failed(FilesystemError::OutsideRoot(path.to_owned())),
            |authority| authority.read_utf8(path, max_bytes, no_symlinks),
        )
    }

    pub(crate) fn inspect(
        &self,
        path: &Path,
        no_symlinks: bool,
    ) -> Result<PathBuf, FilesystemError> {
        let authority = self
            .authority_for_path(path)
            .ok_or_else(|| FilesystemError::OutsideRoot(path.to_owned()))?;
        if no_symlinks {
            authority.inspect_candidate_no_symlinks(path)
        } else {
            authority.inspect_candidate(path)
        }
    }

    pub(crate) fn scan_adoc(
        &self,
        root: &Path,
        max_entries: u64,
        cancellation: &dyn CancellationCheck,
    ) -> Result<ScanResult, FilesystemError> {
        let authority = self
            .authority_for_path(root)
            .ok_or_else(|| FilesystemError::OutsideRoot(root.to_owned()))?;
        let root = authority.inspect_directory_no_symlinks(root)?;
        scan(authority, root, max_entries, cancellation)
    }
}

fn root_error(path: PathBuf, error: FilesystemError) -> FilesystemError {
    match error {
        FilesystemError::Missing(_) => FilesystemError::Missing(path),
        FilesystemError::PermissionDenied(_) => FilesystemError::PermissionDenied(path),
        FilesystemError::OutsideRoot(_) => FilesystemError::OutsideRoot(path),
        FilesystemError::NotDirectory(_) | FilesystemError::NotFile(_) => {
            FilesystemError::NotDirectory(path)
        }
        error => FilesystemError::Inspect {
            path,
            source: error.to_string(),
        },
    }
}

#[cfg(target_os = "linux")]
fn scan(
    authority: &RootAuthority,
    root: PathBuf,
    max_entries: u64,
    cancellation: &dyn CancellationCheck,
) -> Result<ScanResult, FilesystemError> {
    use rustix::fs::{AtFlags, Dir, FileType, statat};
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let mut paths = Vec::new();
    let mut entries = 0_u64;
    let mut directories = 0_u64;
    let mut pending = VecDeque::from([root]);
    let mut complete = true;
    while let Some(directory_path) = pending.pop_front() {
        if cancellation.is_cancelled() {
            return Err(FilesystemError::Unverifiable(
                "local filesystem scan was cancelled".to_owned(),
            ));
        }
        let directory = authority.open_directory_no_symlinks(&directory_path)?;
        directories = directories.saturating_add(1);
        let opened = Dir::read_from(&directory).map_err(|source| FilesystemError::Inspect {
            path: directory_path.clone(),
            source: source.to_string(),
        })?;
        let mut children = Vec::new();
        for child in opened {
            let child = child.map_err(|source| FilesystemError::Inspect {
                path: directory_path.clone(),
                source: source.to_string(),
            })?;
            let name = child.file_name();
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            if entries >= max_entries {
                complete = false;
                break;
            }
            entries += 1;
            let name = OsString::from_vec(name.to_bytes().to_vec());
            let child_path = directory_path.join(&name);
            let metadata =
                statat(&directory, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(|source| {
                    FilesystemError::Inspect {
                        path: child_path.clone(),
                        source: source.to_string(),
                    }
                })?;
            children.push((name, FileType::from_raw_mode(metadata.st_mode)));
        }
        children.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, file_type) in children {
            let child = directory_path.join(name);
            if file_type == FileType::Directory {
                pending.push_back(child);
            } else if file_type == FileType::RegularFile
                && child.extension().and_then(|extension| extension.to_str()) == Some("adoc")
            {
                paths.push(child);
            }
        }
        if !complete {
            break;
        }
    }
    Ok(ScanResult {
        paths,
        entries,
        directories,
        complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use adocweave::NeverCancel;
    use std::fs;

    #[cfg(target_os = "macos")]
    #[test]
    fn the_most_specific_authored_root_selects_its_own_authority() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir_in("/var/tmp").expect("temporary directory");
        let broad = directory.path().join("deep/a/b/broad");
        let narrow = directory.path().join("narrow");
        fs::create_dir_all(&broad).expect("broad root");
        fs::create_dir(&narrow).expect("narrow root");
        fs::write(narrow.join("guide.adoc"), "narrow\n").expect("narrow source");
        let alias = directory.path().join("alias");
        symlink(&broad, &alias).expect("broad alias");
        symlink(&narrow, broad.join("nested")).expect("nested narrow alias");
        let nested_alias = alias.join("nested");
        let authority = FilesystemAuthority::open([alias, nested_alias.clone()])
            .expect("overlapping authored roots");

        let selected = authority
            .authority_for_path(&nested_alias.join("guide.adoc"))
            .expect("narrow authority");
        assert_eq!(
            selected.root(),
            narrow.canonicalize().expect("canonical narrow root")
        );
    }

    #[test]
    fn authority_accepts_128_unique_roots_and_rejects_the_next() {
        let parent = tempfile::tempdir().expect("temporary directory");
        let mut roots = Vec::new();
        for index in 0..129 {
            let root = parent.path().join(index.to_string());
            fs::create_dir(&root).expect("root directory");
            roots.push(root);
        }
        assert!(FilesystemAuthority::open(roots[..128].iter().cloned()).is_ok());
        assert!(matches!(
            FilesystemAuthority::open(roots),
            Err(FilesystemError::LimitExceeded { limit: 128 })
        ));
    }

    #[test]
    fn scan_returns_a_partial_result_at_the_entry_limit() {
        let root = tempfile::tempdir().expect("temporary directory");
        fs::write(root.path().join("b.adoc"), "b").expect("b fixture");
        fs::write(root.path().join("a.adoc"), "a").expect("a fixture");
        let authority = FilesystemAuthority::open([root.path().to_owned()]).expect("authority");
        let result = authority
            .scan_adoc(root.path(), 1, &NeverCancel)
            .expect("partial scan");
        assert!(!result.complete);
        assert_eq!(result.entries, 1);
        assert_eq!(result.paths.len(), 1);
        assert_eq!(
            result.paths[0]
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("adoc")
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_does_not_follow_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary directory");
        let outside = tempfile::tempdir().expect("outside directory");
        fs::write(outside.path().join("outside.adoc"), "outside").expect("outside fixture");
        symlink(outside.path(), root.path().join("linked")).expect("directory symlink");
        let authority = FilesystemAuthority::open([root.path().to_owned()]).expect("authority");
        let result = authority
            .scan_adoc(root.path(), 100, &NeverCancel)
            .expect("scan");
        assert!(result.complete);
        assert!(result.paths.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scan_keeps_the_opened_root_after_its_path_is_replaced() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary directory");
        let outside = tempfile::tempdir().expect("outside directory");
        fs::write(root.path().join("trusted.adoc"), "trusted").expect("trusted fixture");
        fs::write(outside.path().join("outside.adoc"), "outside").expect("outside fixture");
        let authority = FilesystemAuthority::open([root.path().to_owned()]).expect("authority");
        let displaced = root.path().with_extension("displaced-scan");
        fs::rename(root.path(), &displaced).expect("displace root");
        symlink(outside.path(), root.path()).expect("replace root");

        let result = authority
            .scan_adoc(root.path(), 100, &NeverCancel)
            .expect("retained scan");
        assert_eq!(result.paths, vec![root.path().join("trusted.adoc")]);

        fs::remove_file(root.path()).expect("remove replacement");
        fs::rename(displaced, root.path()).expect("restore root");
    }
}

#[cfg(not(target_os = "linux"))]
fn scan(
    _authority: &RootAuthority,
    root: PathBuf,
    max_entries: u64,
    cancellation: &dyn CancellationCheck,
) -> Result<ScanResult, FilesystemError> {
    let mut paths = Vec::new();
    let mut entries = 0_u64;
    let mut directories = 0_u64;
    let mut pending = VecDeque::from([root]);
    let mut complete = true;
    while let Some(directory_path) = pending.pop_front() {
        if cancellation.is_cancelled() {
            return Err(FilesystemError::Unverifiable(
                "local filesystem scan was cancelled".to_owned(),
            ));
        }
        directories = directories.saturating_add(1);
        let mut children = std::fs::read_dir(&directory_path)
            .map_err(|source| FilesystemError::Inspect {
                path: directory_path.clone(),
                source: source.to_string(),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| FilesystemError::Inspect {
                path: directory_path.clone(),
                source: source.to_string(),
            })?;
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            if entries >= max_entries {
                complete = false;
                break;
            }
            entries += 1;
            let path = child.path();
            let file_type = child
                .file_type()
                .map_err(|source| FilesystemError::Inspect {
                    path: path.clone(),
                    source: source.to_string(),
                })?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push_back(path);
            } else if file_type.is_file()
                && path.extension().and_then(|extension| extension.to_str()) == Some("adoc")
            {
                paths.push(path);
            }
        }
        if !complete {
            break;
        }
    }
    Ok(ScanResult {
        paths,
        entries,
        directories,
        complete,
    })
}
