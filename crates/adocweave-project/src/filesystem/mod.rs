use std::fmt;
use std::fs;
use std::io::Read;
use std::io::Write;
#[cfg(target_os = "linux")]
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};

mod authority;
mod error;
mod path;

pub(crate) use authority::FilesystemAuthority;
pub(crate) use error::FilesystemError;
#[cfg(target_os = "linux")]
use path::classify_errno;
use path::{classify_io, decode_relative_path, normalize_below_root};

pub(crate) fn normalize_authored_candidate(
    root: &Path,
    base: &Path,
    target: &str,
) -> Result<PathBuf, FilesystemError> {
    if !base.starts_with(root) {
        return Err(FilesystemError::OutsideRoot(base.to_owned()));
    }
    let relative = decode_relative_path(target)?;
    normalize_below_root(root, base, &relative)
}
#[cfg(not(target_os = "linux"))]
use path::{
    ensure_existing_ancestor_is_inside, normalize_absolute, reject_dangling_symlink_escape,
    reject_symlink_components,
};

/// How many times a confined open may be retried after a concurrent-change race.
///
/// Each retry is a single syscall against a path that is almost always stable,
/// so a small bound absorbs ordinary churn without turning a persistently
/// changing directory into an unbounded wait.
#[cfg(target_os = "linux")]
const CONFINED_OPEN_ATTEMPTS: u32 = 8;

#[cfg(target_os = "linux")]
static NEXT_REPLACEMENT_FILE: AtomicU64 = AtomicU64::new(1);

#[cfg(target_os = "linux")]
struct TemporaryReplacement {
    path: PathBuf,
    file: fs::File,
    committed: bool,
}

#[cfg(target_os = "linux")]
impl Drop for TemporaryReplacement {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
thread_local! {
    static FORCED_OPENAT2_ERROR: std::cell::Cell<Option<rustix::io::Errno>> = const {
        std::cell::Cell::new(None)
    };
    static FORCED_FD_PATH_ERROR: std::cell::Cell<Option<std::io::ErrorKind>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(target_os = "linux")]
fn confined_openat2(
    root: &fs::File,
    relative: &Path,
    flags: rustix::fs::OFlags,
    resolve: rustix::fs::ResolveFlags,
) -> rustix::io::Result<rustix::fd::OwnedFd> {
    #[cfg(test)]
    if let Some(error) = FORCED_OPENAT2_ERROR.with(std::cell::Cell::get) {
        return Err(error);
    }
    rustix::fs::openat2(root, relative, flags, rustix::fs::Mode::empty(), resolve)
}

/// Filesystem boundary for checking an authored relative target.
///
/// The policy owns one canonical project root. It permits parent components
/// only while the normalized path remains below that root, then resolves
/// symlinks before accepting an existing regular file.
/// Equality compares this configured canonical root, not the identity of an
/// operating-system handle acquired by two independently constructed values.
#[derive(Clone)]
pub struct RootAuthority {
    root: PathBuf,
    #[cfg(target_os = "linux")]
    root_handle: Arc<fs::File>,
    #[cfg(not(target_os = "linux"))]
    authored_roots: Vec<PathBuf>,
}

impl fmt::Debug for RootAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RootAuthority")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl PartialEq for RootAuthority {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
    }
}

impl Eq for RootAuthority {}

#[cfg(target_os = "linux")]
struct OpenedTarget {
    canonical_path: PathBuf,
    file: fs::File,
}

impl RootAuthority {
    /// Returns whether both values retain the same filesystem authority.
    ///
    /// This is deliberately distinct from [`PartialEq`], which compares only
    /// the configured canonical root. On Linux, cloned policies share the same
    /// retained directory handle and independently opened policies do not. On
    /// other platforms, the canonical root is the available authority identity.
    pub fn has_same_authority(&self, other: &Self) -> bool {
        if self.root != other.root {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            Arc::ptr_eq(&self.root_handle, &other.root_handle)
        }
        #[cfg(not(target_os = "linux"))]
        {
            true
        }
    }

    pub fn new(root: &Path) -> Result<Self, FilesystemError> {
        #[cfg(target_os = "linux")]
        {
            let (canonical, root_handle) = open_root_directory(root)?;
            Ok(Self {
                root: canonical,
                root_handle: Arc::new(root_handle),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let authored_root = normalize_absolute(root);
            let canonical = root
                .canonicalize()
                .map_err(|source| classify_io(root.to_owned(), source))?;
            if !canonical.is_dir() {
                return Err(FilesystemError::NotDirectory(canonical));
            }
            Ok(Self {
                root: canonical,
                authored_roots: vec![authored_root],
            })
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn contains_path(&self, path: &Path) -> bool {
        self.normalize_path(path).is_ok()
    }

    pub(crate) fn matching_prefix_depth(&self, path: &Path) -> Option<usize> {
        #[cfg(target_os = "linux")]
        {
            path.starts_with(&self.root)
                .then(|| self.root.components().count())
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.normalize_authored_path(path).ok()?;
            std::iter::once(&self.root)
                .chain(self.authored_roots.iter())
                .filter(|root| path.starts_with(root))
                .map(|root| root.components().count())
                .max()
        }
    }

    pub(crate) fn normalize_path(&self, path: &Path) -> Result<PathBuf, FilesystemError> {
        #[cfg(target_os = "linux")]
        {
            path.starts_with(&self.root)
                .then(|| path.to_owned())
                .ok_or_else(|| FilesystemError::OutsideRoot(path.to_owned()))
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.normalize_authored_path(path)
        }
    }

    pub(crate) fn represents_root(&self, path: &Path) -> bool {
        #[cfg(target_os = "linux")]
        {
            path == self.root
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.normalize_authored_path(path)
                .is_ok_and(|normalized| normalized == self.root)
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn merge_authored_roots(&mut self, other: &Self) {
        debug_assert_eq!(self.root, other.root);
        self.authored_roots
            .extend(other.authored_roots.iter().cloned());
        self.authored_roots.sort();
        self.authored_roots.dedup();
    }

    #[cfg(not(target_os = "linux"))]
    fn normalize_authored_path(&self, path: &Path) -> Result<PathBuf, FilesystemError> {
        let rebased = if path.starts_with(&self.root) {
            path.to_owned()
        } else {
            let authored_root = self
                .authored_roots
                .iter()
                .filter(|root| path.starts_with(root))
                .max_by_key(|root| root.components().count())
                .ok_or_else(|| FilesystemError::OutsideRoot(path.to_owned()))?;
            self.root.join(
                path.strip_prefix(authored_root)
                    .expect("a selected authored root is a path prefix"),
            )
        };
        let normalized = normalize_absolute(&rebased);
        if !normalized.starts_with(&self.root) {
            return Err(FilesystemError::OutsideRoot(path.to_owned()));
        }
        Ok(normalized)
    }

    /// Compares a regular file through its retained parent-directory handle.
    ///
    /// Linux resolves the target, temporary file and rename from the retained
    /// parent-directory handle, so replacing an ancestor path cannot redirect
    /// the read. `Ok(false)` reports a concurrent content change.
    #[cfg(target_os = "linux")]
    pub fn candidate_contents_match(
        &self,
        candidate: &Path,
        expected: &[u8],
    ) -> Result<bool, FilesystemError> {
        use std::os::fd::AsRawFd;

        let candidate = self.normalize_candidate(candidate)?;
        let parent = candidate.parent().ok_or_else(|| {
            FilesystemError::Unverifiable("write target has no parent directory".to_owned())
        })?;
        let file_name = candidate.file_name().ok_or_else(|| {
            FilesystemError::Unverifiable("write target has no file name".to_owned())
        })?;
        let directory = self.open_directory_no_symlinks(parent)?;
        let target =
            PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(file_name);
        Ok(read_for_comparison(&target, expected.len(), &candidate)? == expected)
    }

    /// Rechecks a regular file immediately before replacing it atomically.
    ///
    /// Linux resolves the target, temporary file and rename from the retained
    /// parent-directory handle, so replacing an ancestor path cannot redirect
    /// the write. `Ok(false)` reports a content change observed by the final
    /// recheck. The comparison and rename are separate operations, so this is
    /// not a compare-and-swap against writers that ignore this API.
    #[cfg(target_os = "linux")]
    pub fn replace_candidate_after_recheck(
        &self,
        candidate: &Path,
        original: &[u8],
        replacement: &[u8],
    ) -> Result<bool, FilesystemError> {
        use std::os::fd::AsRawFd;

        let candidate = self.normalize_candidate(candidate)?;
        let parent = candidate.parent().ok_or_else(|| {
            FilesystemError::Unverifiable("write target has no parent directory".to_owned())
        })?;
        let file_name = candidate.file_name().ok_or_else(|| {
            FilesystemError::Unverifiable("write target has no file name".to_owned())
        })?;
        let directory = self.open_directory_no_symlinks(parent)?;
        let descriptor_root = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let target = descriptor_root.join(file_name);
        let metadata = fs::symlink_metadata(&target)
            .map_err(|source| classify_io(candidate.clone(), source))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(FilesystemError::NotFile(candidate));
        }
        if read_for_comparison(&target, original.len(), &candidate)? != original {
            return Ok(false);
        }

        let mut temporary = None;
        for _ in 0..CONFINED_OPEN_ATTEMPTS {
            let sequence = NEXT_REPLACEMENT_FILE.fetch_add(1, Ordering::Relaxed);
            let path =
                descriptor_root.join(format!(".adocweave-{}-{sequence}.tmp", std::process::id()));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    temporary = Some(TemporaryReplacement {
                        path,
                        file,
                        committed: false,
                    });
                    break;
                }
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(source) => return Err(classify_io(candidate.clone(), source)),
            }
        }
        let mut temporary = temporary.ok_or_else(|| {
            FilesystemError::Unverifiable(format!(
                "could not create a unique temporary file for {}",
                candidate.display()
            ))
        })?;
        temporary
            .file
            .set_permissions(metadata.permissions())
            .map_err(|source| classify_io(candidate.clone(), source))?;
        temporary
            .file
            .write_all(replacement)
            .and_then(|()| temporary.file.sync_all())
            .map_err(|source| classify_io(candidate.clone(), source))?;
        if read_for_comparison(&target, original.len(), &candidate)? != original {
            return Ok(false);
        }
        fs::rename(&temporary.path, &target)
            .map_err(|source| classify_io(candidate.clone(), source))?;
        temporary.committed = true;
        directory
            .sync_all()
            .map_err(|source| classify_io(candidate, source))?;
        Ok(true)
    }

    /// Compares a regular file after rejecting symbolic links in its path.
    #[cfg(not(target_os = "linux"))]
    pub fn candidate_contents_match(
        &self,
        candidate: &Path,
        expected: &[u8],
    ) -> Result<bool, FilesystemError> {
        let candidate = self.inspect_candidate_no_symlinks(candidate)?;
        Ok(read_for_comparison(&candidate, expected.len(), &candidate)? == expected)
    }

    /// Rechecks and replaces a regular file through a temporary file in the
    /// same directory.
    ///
    /// Platforms without handle-relative path resolution reject symbolic links
    /// immediately before each comparison. This cannot prevent every path race,
    /// so callers must treat the result as valid only for a static snapshot.
    #[cfg(not(target_os = "linux"))]
    pub fn replace_candidate_after_recheck(
        &self,
        candidate: &Path,
        original: &[u8],
        replacement: &[u8],
    ) -> Result<bool, FilesystemError> {
        let candidate = self.inspect_candidate_no_symlinks(candidate)?;
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|source| classify_io(candidate.clone(), source))?;
        if read_for_comparison(&candidate, original.len(), &candidate)? != original {
            return Ok(false);
        }
        let parent = candidate.parent().ok_or_else(|| {
            FilesystemError::Unverifiable("write target has no parent directory".to_owned())
        })?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|source| classify_io(candidate.clone(), source))?;
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .and_then(|_| temporary.write_all(replacement))
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|source| classify_io(candidate.clone(), source))?;
        let rechecked = self.inspect_candidate_no_symlinks(&candidate)?;
        if read_for_comparison(&rechecked, original.len(), &candidate)? != original {
            return Ok(false);
        }
        temporary
            .persist(&candidate)
            .map_err(|error| classify_io(candidate.clone(), error.error))?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| classify_io(candidate, source))?;
        Ok(true)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn root_directory_handle(&self) -> Result<fs::File, FilesystemError> {
        use rustix::fs::{Mode, OFlags, openat};

        openat(
            self.root_handle.as_ref(),
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(fs::File::from)
        .map_err(|error| classify_errno(&self.root, error))
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn open_directory_no_symlinks(
        &self,
        candidate: &Path,
    ) -> Result<fs::File, FilesystemError> {
        use rustix::fd::OwnedFd;
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat};

        let relative = candidate
            .strip_prefix(&self.root)
            .map_err(|_| FilesystemError::OutsideRoot(candidate.to_owned()))?;
        if relative.as_os_str().is_empty() {
            return self.root_directory_handle();
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut attempts = 0;
        let directory = loop {
            let outcome = confined_openat2(
                self.root_handle.as_ref(),
                relative,
                flags,
                ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS,
            );
            attempts += 1;
            if matches!(outcome, Err(rustix::io::Errno::AGAIN)) && attempts < CONFINED_OPEN_ATTEMPTS
            {
                continue;
            }
            break outcome;
        };
        let directory = match directory {
            Ok(directory) => directory,
            Err(error)
                if error == rustix::io::Errno::NOSYS || error == rustix::io::Errno::INVAL =>
            {
                let mut directory: OwnedFd = openat(
                    self.root_handle.as_ref(),
                    ".",
                    OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| classify_errno(candidate, error))?;
                let mut components = relative.components().peekable();
                while let Some(component) = components.next() {
                    let Component::Normal(name) = component else {
                        return Err(FilesystemError::Unverifiable(
                            candidate.to_string_lossy().into_owned(),
                        ));
                    };
                    let component_flags = if components.peek().is_some() {
                        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
                    } else {
                        flags
                    };
                    directory = openat(&directory, name, component_flags, Mode::empty())
                        .map_err(|error| classify_errno(candidate, error))?;
                }
                directory
            }
            Err(error) => return Err(classify_errno(candidate, error)),
        };
        Ok(fs::File::from(directory))
    }

    /// Derives a directory policy below this already opened root.
    ///
    /// The authored path must not contain a symbolic-link hop. Keeping the
    /// candidate spelling while opening it relative to `root_handle` lets the
    /// derived policy continue to name the same logical namespace after the
    /// root's directory entry is concurrently replaced.
    pub fn derive_confined_directory(&self, candidate: &Path) -> Result<Self, FilesystemError> {
        #[cfg(target_os = "linux")]
        {
            let relative = candidate
                .strip_prefix(&self.root)
                .map_err(|_| FilesystemError::OutsideRoot(candidate.to_owned()))?;
            let logical_root = self.root.join(relative);
            let root_handle = self.open_directory_no_symlinks(&logical_root)?;
            Ok(Self {
                root: logical_root,
                root_handle: Arc::new(root_handle),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let candidate = self.normalize_authored_path(candidate)?;
            candidate
                .strip_prefix(&self.root)
                .map_err(|_| FilesystemError::OutsideRoot(candidate.to_owned()))?;
            reject_symlink_components(&self.root, &candidate)?;
            let canonical = candidate
                .canonicalize()
                .map_err(|source| classify_io(candidate.to_owned(), source))?;
            if !canonical.starts_with(&self.root) {
                return Err(FilesystemError::OutsideRoot(canonical));
            }
            if !canonical.is_dir() {
                return Err(FilesystemError::NotDirectory(canonical));
            }
            let relative = canonical
                .strip_prefix(&self.root)
                .expect("a confined canonical directory remains below its authority");
            let mut authored_roots = self
                .authored_roots
                .iter()
                .map(|root| root.join(relative))
                .collect::<Vec<_>>();
            authored_roots.push(canonical.clone());
            authored_roots.sort();
            authored_roots.dedup();
            Ok(Self {
                root: canonical,
                authored_roots,
            })
        }
    }

    pub fn inspect(&self, base: &Path, target: &str) -> Result<PathBuf, FilesystemError> {
        let candidate = self.candidate(base, target)?;
        self.inspect_candidate(&candidate)
    }

    /// Resolves URL path syntax and parent components without touching the target.
    ///
    /// Callers may use the returned normalized path as a per-run cache key.
    pub fn candidate(&self, base: &Path, target: &str) -> Result<PathBuf, FilesystemError> {
        let base = self.inspect_directory_no_symlinks(base)?;
        self.candidate_from_verified_base(&base, target)
    }

    /// Normalizes an absolute logical path without consulting the current
    /// process namespace.
    ///
    /// Parent components may move within this policy's root but never above
    /// it. The result is suitable for handle-relative inspection, including
    /// paths whose final component does not exist yet.
    #[cfg(target_os = "linux")]
    pub fn normalize_candidate(&self, candidate: &Path) -> Result<PathBuf, FilesystemError> {
        let relative = candidate
            .strip_prefix(&self.root)
            .map_err(|_| FilesystemError::OutsideRoot(candidate.to_owned()))?;
        let mut normalized = self.root.clone();
        let mut depth = 0_usize;
        for component in relative.components() {
            match component {
                Component::Normal(name) => {
                    normalized.push(name);
                    depth = depth.saturating_add(1);
                }
                Component::CurDir => {}
                Component::ParentDir if depth > 0 => {
                    normalized.pop();
                    depth -= 1;
                }
                Component::ParentDir => {
                    return Err(FilesystemError::OutsideRoot(candidate.to_owned()));
                }
                Component::Prefix(_) | Component::RootDir => {
                    return Err(FilesystemError::Unverifiable(
                        candidate.to_string_lossy().into_owned(),
                    ));
                }
            }
        }
        Ok(normalized)
    }

    fn candidate_from_verified_base(
        &self,
        base: &Path,
        target: &str,
    ) -> Result<PathBuf, FilesystemError> {
        normalize_authored_candidate(&self.root, base, target)
    }

    #[cfg(target_os = "linux")]
    pub fn inspect_candidate(&self, candidate: &Path) -> Result<PathBuf, FilesystemError> {
        if !candidate.starts_with(&self.root) {
            return Err(FilesystemError::OutsideRoot(candidate.to_owned()));
        }
        Ok(self.open_confined(candidate)?.canonical_path)
    }

    /// Resolves a normalized regular file while rejecting every symbolic link.
    pub fn inspect_candidate_no_symlinks(
        &self,
        candidate: &Path,
    ) -> Result<PathBuf, FilesystemError> {
        #[cfg(target_os = "linux")]
        {
            if !candidate.starts_with(&self.root) {
                return Err(FilesystemError::OutsideRoot(candidate.to_owned()));
            }
            self.open_confined_with_symlinks(candidate, false)
                .map(|opened| opened.canonical_path)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let candidate = self.normalize_authored_path(candidate)?;
            reject_symlink_components(&self.root, &candidate)?;
            self.inspect_candidate(&candidate)
        }
    }

    pub(crate) fn read_utf8(
        &self,
        candidate: &Path,
        max_bytes: u64,
        no_symlinks: bool,
    ) -> FilesystemRead {
        #[cfg(target_os = "linux")]
        {
            self.read_utf8_after_open(candidate, max_bytes, no_symlinks, || {})
        }
        #[cfg(not(target_os = "linux"))]
        {
            let opened = if no_symlinks {
                self.inspect_candidate_no_symlinks(candidate)
            } else {
                self.inspect_candidate(candidate)
            }
            .and_then(|canonical| {
                fs::File::open(&canonical)
                    .map(|file| (canonical, file))
                    .map_err(|source| classify_io(candidate.to_owned(), source))
            });
            match opened {
                Ok((canonical, file)) => read_bounded_utf8(file, canonical, max_bytes),
                Err(error) => FilesystemRead::failed(error),
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn read_utf8_after_open(
        &self,
        candidate: &Path,
        max_bytes: u64,
        no_symlinks: bool,
        after_open: impl FnOnce(),
    ) -> FilesystemRead {
        match self.open_confined_with_symlinks_after_open(candidate, !no_symlinks, after_open) {
            Ok(opened) => read_bounded_utf8(opened.file, opened.canonical_path, max_bytes),
            Err(error) => FilesystemRead::failed(error),
        }
    }

    /// Resolves an existing directory without crossing a symbolic link.
    ///
    /// On handle-relative platforms the returned path remains in the logical
    /// namespace established when this policy was created.
    pub fn inspect_directory_no_symlinks(
        &self,
        candidate: &Path,
    ) -> Result<PathBuf, FilesystemError> {
        #[cfg(target_os = "linux")]
        {
            let relative = candidate
                .strip_prefix(&self.root)
                .map_err(|_| FilesystemError::OutsideRoot(candidate.to_owned()))?;
            self.open_directory_no_symlinks(candidate)?;
            Ok(self.root.join(relative))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let candidate = self.normalize_authored_path(candidate)?;
            reject_symlink_components(&self.root, &candidate)?;
            let canonical = candidate
                .canonicalize()
                .map_err(|source| classify_io(candidate.to_owned(), source))?;
            if !canonical.starts_with(&self.root) {
                return Err(FilesystemError::OutsideRoot(canonical));
            }
            if !canonical.is_dir() {
                return Err(FilesystemError::NotDirectory(canonical));
            }
            Ok(canonical)
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn inspect_candidate(&self, candidate: &Path) -> Result<PathBuf, FilesystemError> {
        let candidate = self.normalize_authored_path(candidate)?;
        let canonical = match candidate.canonicalize() {
            Ok(path) => path,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                reject_dangling_symlink_escape(&self.root, &candidate)?;
                ensure_existing_ancestor_is_inside(&self.root, &candidate)?;
                return Err(FilesystemError::Missing(candidate.to_owned()));
            }
            Err(source) => return Err(classify_io(candidate.to_owned(), source)),
        };
        if !canonical.starts_with(&self.root) {
            return Err(FilesystemError::OutsideRoot(canonical));
        }
        let metadata =
            fs::metadata(&canonical).map_err(|source| classify_io(canonical.clone(), source))?;
        if !metadata.is_file() {
            return Err(FilesystemError::NotFile(canonical));
        }
        Ok(canonical)
    }

    #[cfg(target_os = "linux")]
    fn open_confined(&self, candidate: &Path) -> Result<OpenedTarget, FilesystemError> {
        self.open_confined_with_symlinks(candidate, true)
    }

    #[cfg(target_os = "linux")]
    fn open_confined_with_symlinks(
        &self,
        candidate: &Path,
        follow_symlinks: bool,
    ) -> Result<OpenedTarget, FilesystemError> {
        self.open_confined_with_symlinks_after_open(candidate, follow_symlinks, || {})
    }

    #[cfg(target_os = "linux")]
    fn open_confined_with_symlinks_after_open(
        &self,
        candidate: &Path,
        follow_symlinks: bool,
        after_open: impl FnOnce(),
    ) -> Result<OpenedTarget, FilesystemError> {
        use rustix::fd::OwnedFd;
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat};

        let relative = candidate
            .strip_prefix(&self.root)
            .map_err(|_| FilesystemError::OutsideRoot(candidate.to_owned()))?;
        let root = self.root_handle.as_ref();
        // Opening a FIFO for reading can otherwise wait for a writer before we
        // have a handle whose file type can be rejected.
        let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC;
        // `RESOLVE_BENEATH` makes the kernel give up with `EAGAIN` when another
        // process renames or mounts something along this path while it is being
        // resolved. The lookup was neither denied nor granted, so the only
        // correct response is to look again. The attempt count is bounded so a
        // filesystem under constant churn fails instead of spinning.
        let mut attempts = 0;
        let file = loop {
            let mut resolve = ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS;
            if !follow_symlinks {
                resolve |= ResolveFlags::NO_SYMLINKS;
            }
            let outcome = confined_openat2(root, relative, flags, resolve);
            attempts += 1;
            if matches!(outcome, Err(rustix::io::Errno::AGAIN)) && attempts < CONFINED_OPEN_ATTEMPTS
            {
                continue;
            }
            break outcome;
        };
        let file = match file {
            Ok(file) => fs::File::from(file),
            Err(error)
                if error == rustix::io::Errno::NOSYS || error == rustix::io::Errno::INVAL =>
            {
                let mut directory: OwnedFd = openat(
                    root,
                    ".",
                    OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| classify_errno(candidate, error))?;
                let mut components = relative.components().peekable();
                while let Some(component) = components.next() {
                    let Component::Normal(name) = component else {
                        return Err(FilesystemError::Unverifiable(
                            candidate.to_string_lossy().into_owned(),
                        ));
                    };
                    let component_flags = if components.peek().is_some() {
                        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
                    } else {
                        flags | OFlags::NOFOLLOW
                    };
                    directory = openat(&directory, name, component_flags, Mode::empty())
                        .map_err(|error| classify_errno(candidate, error))?;
                }
                fs::File::from(directory)
            }
            Err(error) => return Err(classify_errno(candidate, error)),
        };
        if !file
            .metadata()
            .map_err(|source| classify_io(candidate.to_owned(), source))?
            .is_file()
        {
            return Err(FilesystemError::NotFile(candidate.to_owned()));
        }
        after_open();
        let canonical_path = logical_path_from_opened_handle(
            &self.root,
            self.root_handle.as_ref(),
            &file,
            candidate,
        )?;
        Ok(OpenedTarget {
            canonical_path,
            file,
        })
    }
}

#[cfg(target_os = "linux")]
fn logical_path_from_opened_handle(
    logical_root: &Path,
    root_handle: &fs::File,
    opened: &fs::File,
    candidate: &Path,
) -> Result<PathBuf, FilesystemError> {
    logical_path_from_opened_handle_with(logical_root, root_handle, opened, candidate, || {}, || {})
}

#[cfg(target_os = "linux")]
fn logical_path_from_opened_handle_with(
    logical_root: &Path,
    root_handle: &fs::File,
    opened: &fs::File,
    candidate: &Path,
    after_root_path: impl FnOnce(),
    before_identity_open: impl FnOnce(),
) -> Result<PathBuf, FilesystemError> {
    let root_path = path_from_fd(root_handle, candidate)?;
    after_root_path();
    let opened_path = path_from_fd(opened, candidate)?;
    let relative = opened_path
        .strip_prefix(&root_path)
        .map_err(|_| FilesystemError::Unverifiable(candidate.to_string_lossy().into_owned()))?;
    before_identity_open();
    verify_opened_path_identity(root_handle, opened, relative, candidate)?;
    Ok(logical_root.join(relative))
}

#[cfg(target_os = "linux")]
fn verify_opened_path_identity(
    root_handle: &fs::File,
    opened: &fs::File,
    relative: &Path,
    candidate: &Path,
) -> Result<(), FilesystemError> {
    use std::os::unix::fs::MetadataExt;

    use rustix::fd::OwnedFd;
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat};

    let flags = OFlags::PATH | OFlags::CLOEXEC;
    let mut attempts = 0;
    let verified = loop {
        let outcome = confined_openat2(
            root_handle,
            relative,
            flags,
            ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS,
        );
        attempts += 1;
        if matches!(outcome, Err(rustix::io::Errno::AGAIN)) && attempts < CONFINED_OPEN_ATTEMPTS {
            continue;
        }
        break outcome;
    };
    let verified = match verified {
        Ok(file) => fs::File::from(file),
        Err(error) if error == rustix::io::Errno::NOSYS || error == rustix::io::Errno::INVAL => {
            let mut directory: OwnedFd = openat(
                root_handle,
                ".",
                OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| opened_identity_error(candidate, error))?;
            let mut components = relative.components().peekable();
            while let Some(component) = components.next() {
                let Component::Normal(name) = component else {
                    return Err(opened_identity_mismatch(candidate));
                };
                let component_flags = if components.peek().is_some() {
                    OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
                } else {
                    flags | OFlags::NOFOLLOW
                };
                directory = openat(&directory, name, component_flags, Mode::empty())
                    .map_err(|error| opened_identity_error(candidate, error))?;
            }
            fs::File::from(directory)
        }
        Err(error) => return Err(opened_identity_error(candidate, error)),
    };
    let opened_metadata = opened
        .metadata()
        .map_err(|error| opened_identity_io_error(candidate, error))?;
    let verified_metadata = verified
        .metadata()
        .map_err(|error| opened_identity_io_error(candidate, error))?;
    if opened_metadata.dev() != verified_metadata.dev()
        || opened_metadata.ino() != verified_metadata.ino()
        || opened_metadata.file_type() != verified_metadata.file_type()
    {
        return Err(opened_identity_mismatch(candidate));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn opened_identity_error(candidate: &Path, source: rustix::io::Errno) -> FilesystemError {
    FilesystemError::Unverifiable(format!(
        "cannot verify the opened local target path for {}: {source}",
        candidate.display()
    ))
}

#[cfg(target_os = "linux")]
fn opened_identity_io_error(candidate: &Path, source: std::io::Error) -> FilesystemError {
    FilesystemError::Unverifiable(format!(
        "cannot verify the opened local target identity for {}: {source}",
        candidate.display()
    ))
}

#[cfg(target_os = "linux")]
fn opened_identity_mismatch(candidate: &Path) -> FilesystemError {
    FilesystemError::Unverifiable(format!(
        "opened local target no longer matches its filesystem path: {}",
        candidate.display()
    ))
}

#[cfg(target_os = "linux")]
fn path_from_fd(file: &fs::File, candidate: &Path) -> Result<PathBuf, FilesystemError> {
    use std::os::fd::AsRawFd;

    #[cfg(test)]
    if let Some(kind) = FORCED_FD_PATH_ERROR.with(std::cell::Cell::get) {
        return Err(fd_path_error(candidate, std::io::Error::from(kind)));
    }

    fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
        .map_err(|source| fd_path_error(candidate, source))
}

#[cfg(target_os = "linux")]
fn fd_path_error(candidate: &Path, source: std::io::Error) -> FilesystemError {
    FilesystemError::Unverifiable(format!(
        "cannot resolve the opened local target through /proc/self/fd for {}: {source}",
        candidate.display()
    ))
}

fn read_bounded_utf8(file: impl Read, canonical_path: PathBuf, max_bytes: u64) -> FilesystemRead {
    let (bytes, read_error) = read_bounded_bytes(file, max_bytes);
    let observed_bytes = bytes.len() as u64;
    if let Some(source) = read_error {
        return FilesystemRead {
            outcome: Err(classify_io(canonical_path, source)),
            observed_bytes,
        };
    }
    if bytes.len() as u64 > max_bytes {
        return FilesystemRead {
            outcome: Err(FilesystemError::ResourceTooLarge(canonical_path)),
            observed_bytes,
        };
    }
    let outcome = String::from_utf8(bytes)
        .map(|source| LoadedText {
            canonical_path: canonical_path.clone(),
            source: Arc::from(source),
        })
        .map_err(|_| FilesystemError::InvalidUtf8(canonical_path));
    FilesystemRead {
        outcome,
        observed_bytes,
    }
}

pub(crate) struct FilesystemRead {
    pub(crate) outcome: Result<LoadedText, FilesystemError>,
    pub(crate) observed_bytes: u64,
}

impl FilesystemRead {
    pub(super) fn failed(error: FilesystemError) -> Self {
        Self {
            outcome: Err(error),
            observed_bytes: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoadedText {
    canonical_path: PathBuf,
    source: Arc<str>,
}

impl LoadedText {
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }
}

/// Reads at most one byte beyond `max_bytes` so the caller can tell an exactly
/// sized resource from an oversized one, and counts what was obtained even when
/// the read fails part of the way through.
fn read_bounded_bytes(reader: impl Read, max_bytes: u64) -> (Vec<u8>, Option<std::io::Error>) {
    let mut bytes = Vec::new();
    let result = reader
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes);
    (bytes, result.err())
}

#[cfg(target_os = "linux")]
fn read_for_comparison(
    path: &Path,
    expected_len: usize,
    logical_path: &Path,
) -> Result<Vec<u8>, FilesystemError> {
    use rustix::fs::{Mode, OFlags, open};

    let file = open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(fs::File::from)
    .map_err(|error| classify_errno(logical_path, error))?;
    if !file
        .metadata()
        .map_err(|source| classify_io(logical_path.to_owned(), source))?
        .is_file()
    {
        return Err(FilesystemError::NotFile(logical_path.to_owned()));
    }
    let limit = u64::try_from(expected_len)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut current = Vec::with_capacity(expected_len.saturating_add(1));
    file.take(limit)
        .read_to_end(&mut current)
        .map_err(|source| classify_io(logical_path.to_owned(), source))?;
    Ok(current)
}

#[cfg(not(target_os = "linux"))]
fn read_for_comparison(
    path: &Path,
    expected_len: usize,
    logical_path: &Path,
) -> Result<Vec<u8>, FilesystemError> {
    let file =
        fs::File::open(path).map_err(|source| classify_io(logical_path.to_owned(), source))?;
    if !file
        .metadata()
        .map_err(|source| classify_io(logical_path.to_owned(), source))?
        .is_file()
    {
        return Err(FilesystemError::NotFile(logical_path.to_owned()));
    }
    let limit = u64::try_from(expected_len)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut current = Vec::with_capacity(expected_len.saturating_add(1));
    file.take(limit)
        .read_to_end(&mut current)
        .map_err(|source| classify_io(logical_path.to_owned(), source))?;
    Ok(current)
}

#[cfg(target_os = "linux")]
fn open_root_directory(root: &Path) -> Result<(PathBuf, fs::File), FilesystemError> {
    use rustix::fs::{Mode, OFlags, open};

    let file = open(
        root,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(fs::File::from)
    .map_err(|error| {
        if error == rustix::io::Errno::NOTDIR {
            FilesystemError::NotDirectory(root.to_owned())
        } else {
            classify_errno(root, error)
        }
    })?;
    let canonical = path_from_fd(&file, root)?;
    Ok((canonical, file))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PartialReader {
        emitted: bool,
    }

    impl Read for PartialReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.emitted {
                return Err(std::io::Error::other("injected partial read failure"));
            }
            self.emitted = true;
            buffer[..3].copy_from_slice(b"abc");
            Ok(3)
        }
    }

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temporary directory");
        fs::create_dir(root.path().join("docs")).expect("docs directory");
        fs::write(root.path().join("docs/guide.adoc"), "= Guide").expect("guide fixture");
        root
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn authored_var_alias_uses_the_canonical_root_without_widening_authority() {
        use std::os::unix::fs::symlink;

        let root = tempfile::Builder::new()
            .prefix("adocweave-authority-")
            .tempdir_in("/var/tmp")
            .expect("temporary authority root");
        let canonical = root
            .path()
            .canonicalize()
            .expect("canonical authority root");
        let authored = Path::new("/var").join(
            canonical
                .strip_prefix("/private/var")
                .expect("macOS /var resolves below /private/var"),
        );
        fs::write(authored.join("guide.adoc"), "authored\n").expect("authored source");
        let authority = RootAuthority::new(&authored).expect("authority from authored spelling");

        assert!(matches!(
            authority
                .read_utf8(&authored.join("guide.adoc"), 1024, false)
                .outcome,
            Ok(ref source) if source.source() == "authored\n"
        ));

        let outside = tempfile::tempdir_in("/var/tmp").expect("outside directory");
        fs::write(outside.path().join("secret.adoc"), "secret\n").expect("outside source");
        symlink(
            outside.path().join("secret.adoc"),
            authored.join("linked.adoc"),
        )
        .expect("outside symlink");
        assert!(matches!(
            authority.inspect_candidate(&authored.join("linked.adoc")),
            Err(FilesystemError::OutsideRoot(_))
        ));
        assert!(matches!(
            authority.inspect_candidate(&outside.path().join("secret.adoc")),
            Err(FilesystemError::OutsideRoot(_))
        ));
    }

    #[test]
    fn path_normalization_rejects_escape_and_decodes_a_relative_target() {
        let root = fixture();
        let authority = RootAuthority::new(root.path()).expect("authority");
        assert_eq!(
            authority
                .candidate(&root.path().join("docs"), "%67uide.adoc")
                .expect("decoded target"),
            root.path().join("docs/guide.adoc")
        );
        assert!(matches!(
            authority.candidate(&root.path().join("docs"), "../../outside.adoc"),
            Err(FilesystemError::OutsideRoot(_))
        ));
    }

    #[test]
    fn bounded_utf8_read_rejects_invalid_and_oversized_input() {
        let root = fixture();
        fs::write(root.path().join("docs/invalid.adoc"), [0xff]).expect("invalid fixture");
        let authority = RootAuthority::new(root.path()).expect("authority");
        assert!(matches!(
            authority
                .read_utf8(&root.path().join("docs/invalid.adoc"), 16, false)
                .outcome,
            Err(FilesystemError::InvalidUtf8(_))
        ));
        assert!(matches!(
            authority
                .read_utf8(&root.path().join("docs/guide.adoc"), 2, false)
                .outcome,
            Err(FilesystemError::ResourceTooLarge(_))
        ));
    }

    #[test]
    fn bounded_read_reports_bytes_observed_before_an_io_failure() {
        let read = read_bounded_utf8(
            PartialReader { emitted: false },
            PathBuf::from("/project/partial.adoc"),
            16,
        );
        assert_eq!(read.observed_bytes, 3);
        assert!(matches!(
            read.outcome,
            Err(FilesystemError::Unverifiable(_))
        ));
    }

    #[test]
    fn cloned_authority_keeps_its_handle_identity() {
        let root = fixture();
        let authority = RootAuthority::new(root.path()).expect("authority");
        assert!(authority.has_same_authority(&authority.clone()));
    }

    #[cfg(unix)]
    #[test]
    fn confined_authority_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = fixture();
        let outside = tempfile::tempdir().expect("outside directory");
        symlink(outside.path(), root.path().join("docs/outside")).expect("symlink");
        let authority = RootAuthority::new(root.path()).expect("authority");
        assert!(matches!(
            authority.inspect(&root.path().join("docs"), "outside/missing.adoc"),
            Err(FilesystemError::OutsideRoot(_))
        ));
    }

    #[test]
    fn safe_replace_rechecks_the_original_body() {
        let root = fixture();
        let target = root.path().join("docs/guide.adoc");
        let authority = RootAuthority::new(root.path()).expect("authority");
        assert!(
            authority
                .candidate_contents_match(&target, b"= Guide")
                .expect("comparison")
        );
        assert!(
            !authority
                .replace_candidate_after_recheck(&target, b"changed", b"replacement")
                .expect("changed body")
        );
        assert!(
            authority
                .replace_candidate_after_recheck(&target, b"= Guide", b"= Replaced")
                .expect("replace body")
        );
        assert_eq!(
            fs::read_to_string(target).expect("replacement"),
            "= Replaced"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn retained_root_handle_does_not_follow_a_replacement_symlink() {
        use std::os::unix::fs::symlink;

        let root = fixture();
        let outside = fixture();
        fs::write(outside.path().join("docs/guide.adoc"), "= Outside").expect("outside body");
        let authority = RootAuthority::new(root.path()).expect("authority");
        let displaced = root.path().with_extension("displaced");
        fs::rename(root.path(), &displaced).expect("displace root");
        symlink(outside.path(), root.path()).expect("replacement root");

        let loaded = authority
            .read_utf8(&root.path().join("docs/guide.adoc"), 1024, false)
            .outcome
            .expect("retained read");
        assert_eq!(loaded.source(), "= Guide");

        fs::remove_file(root.path()).expect("remove replacement");
        fs::rename(displaced, root.path()).expect("restore root");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn opened_file_is_stable_when_an_ancestor_is_replaced() {
        use std::os::unix::fs::symlink;

        let root = fixture();
        let outside = tempfile::tempdir().expect("outside directory");
        fs::write(outside.path().join("guide.adoc"), "= Outside").expect("outside body");
        let authority = RootAuthority::new(root.path()).expect("authority");
        let docs = root.path().join("docs");
        let displaced = root.path().join("opened-docs");
        let target = docs.join("guide.adoc");

        let loaded = authority
            .read_utf8_after_open(&target, 1024, false, || {
                fs::rename(&docs, &displaced).expect("displace ancestor");
                symlink(outside.path(), &docs).expect("replacement ancestor");
            })
            .outcome
            .expect("retained contents");
        assert_eq!(loaded.source(), "= Guide");
        assert_ne!(loaded.source(), "= Outside");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn opened_file_is_stable_when_the_leaf_is_replaced() {
        let root = fixture();
        let authority = RootAuthority::new(root.path()).expect("authority");
        let target = root.path().join("docs/guide.adoc");
        let displaced = root.path().join("docs/original.adoc");

        let loaded = authority
            .read_utf8_after_open(&target, 1024, false, || {
                fs::rename(&target, &displaced).expect("displace leaf");
                fs::write(&target, "= Replacement").expect("replacement leaf");
            })
            .outcome
            .expect("retained contents");
        assert_eq!(loaded.source(), "= Guide");
        assert_eq!(loaded.canonical_path(), displaced);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn openat2_fallback_remains_handle_relative() {
        let root = fixture();
        let authority = RootAuthority::new(root.path()).expect("authority");
        FORCED_OPENAT2_ERROR.with(|forced| forced.set(Some(rustix::io::Errno::NOSYS)));
        let loaded = authority
            .read_utf8(&root.path().join("docs/guide.adoc"), 1024, false)
            .outcome
            .expect("fallback read");
        FORCED_OPENAT2_ERROR.with(|forced| forced.set(None));
        assert_eq!(loaded.source(), "= Guide");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn openat2_fallback_verifies_rename_and_rejects_unlink() {
        let renamed_root = fixture();
        let renamed_authority = RootAuthority::new(renamed_root.path()).expect("authority");
        let renamed_target = renamed_root.path().join("docs/guide.adoc");
        let displaced = renamed_root.path().join("docs/original.adoc");
        FORCED_OPENAT2_ERROR.with(|forced| forced.set(Some(rustix::io::Errno::NOSYS)));
        let renamed = renamed_authority.read_utf8_after_open(&renamed_target, 1024, false, || {
            fs::rename(&renamed_target, &displaced).expect("rename opened target")
        });
        FORCED_OPENAT2_ERROR.with(|forced| forced.set(None));
        let renamed = renamed.outcome.expect("renamed inode is verified");
        assert_eq!(renamed.canonical_path(), displaced);
        assert_eq!(renamed.source(), "= Guide");

        let unlinked_root = fixture();
        let unlinked_authority = RootAuthority::new(unlinked_root.path()).expect("authority");
        let unlinked_target = unlinked_root.path().join("docs/guide.adoc");
        FORCED_OPENAT2_ERROR.with(|forced| forced.set(Some(rustix::io::Errno::NOSYS)));
        let unlinked =
            unlinked_authority.read_utf8_after_open(&unlinked_target, 1024, false, || {
                fs::remove_file(&unlinked_target).expect("unlink opened target")
            });
        FORCED_OPENAT2_ERROR.with(|forced| forced.set(None));
        assert!(matches!(
            unlinked.outcome,
            Err(FilesystemError::Unverifiable(_))
        ));
        assert_eq!(unlinked.observed_bytes, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn procfs_failure_closes_authority_creation() {
        let root = fixture();
        FORCED_FD_PATH_ERROR.with(|forced| forced.set(Some(std::io::ErrorKind::PermissionDenied)));
        let error = RootAuthority::new(root.path()).expect_err("procfs failure");
        FORCED_FD_PATH_ERROR.with(|forced| forced.set(None));
        assert!(matches!(error, FilesystemError::Unverifiable(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn procfs_failure_after_authority_creation_fails_closed() {
        let root = fixture();
        let authority = RootAuthority::new(root.path()).expect("authority");
        FORCED_FD_PATH_ERROR.with(|forced| forced.set(Some(std::io::ErrorKind::NotFound)));
        let error = authority
            .inspect_candidate(&root.path().join("docs/guide.adoc"))
            .expect_err("procfs failure");
        FORCED_FD_PATH_ERROR.with(|forced| forced.set(None));
        assert!(matches!(error, FilesystemError::Unverifiable(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fifo_is_rejected_without_opening_its_body() {
        use rustix::fs::{CWD, Mode, mkfifoat};

        let root = fixture();
        let fifo = root.path().join("docs/input.adoc");
        mkfifoat(CWD, &fifo, Mode::RUSR | Mode::WUSR).expect("FIFO");
        let authority = RootAuthority::new(root.path()).expect("authority");
        assert!(matches!(
            authority.read_utf8(&fifo, 1024, false).outcome,
            Err(FilesystemError::NotFile(path)) if path == fifo
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn literal_deleted_suffix_is_not_treated_as_an_unlinked_file() {
        let root = fixture();
        let target = root.path().join("docs/guide.adoc (deleted)");
        fs::write(&target, "literal suffix").expect("suffix body");
        let authority = RootAuthority::new(root.path()).expect("authority");
        let loaded = authority
            .read_utf8(&target, 1024, false)
            .outcome
            .expect("suffix read");
        assert_eq!(loaded.canonical_path(), target);
        assert_eq!(loaded.source(), "literal suffix");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unlinked_file_cannot_collide_with_a_literal_deleted_suffix() {
        let root = fixture();
        let target = root.path().join("docs/guide.adoc");
        fs::write(root.path().join("docs/guide.adoc (deleted)"), "collision")
            .expect("collision file");
        let authority = RootAuthority::new(root.path()).expect("authority");
        let read = authority.read_utf8_after_open(&target, 1024, false, || {
            fs::remove_file(&target).expect("unlink opened target");
        });
        assert!(matches!(
            read.outcome,
            Err(FilesystemError::Unverifiable(_))
        ));
        assert_eq!(read.observed_bytes, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn identity_reopen_rejects_a_leaf_swap() {
        let root = fixture();
        let authority = RootAuthority::new(root.path()).expect("authority");
        let target = root.path().join("docs/guide.adoc");
        let displaced = root.path().join("docs/original.adoc");
        let opened = fs::File::open(&target).expect("opened target");
        let result = logical_path_from_opened_handle_with(
            authority.root(),
            authority.root_handle.as_ref(),
            &opened,
            &target,
            || {},
            || {
                fs::rename(&target, &displaced).expect("displace resolved target");
                fs::write(&target, "= Replacement").expect("replacement target");
            },
        );
        assert!(matches!(result, Err(FilesystemError::Unverifiable(_))));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn root_rename_between_descriptor_paths_fails_closed() {
        let root = fixture();
        let authority = RootAuthority::new(root.path()).expect("authority");
        let target = root.path().join("docs/guide.adoc");
        let opened = fs::File::open(&target).expect("opened target");
        let displaced = root.path().with_extension("between-descriptor-reads");
        let result = logical_path_from_opened_handle_with(
            authority.root(),
            authority.root_handle.as_ref(),
            &opened,
            &target,
            || fs::rename(root.path(), &displaced).expect("rename root"),
            || {},
        );
        fs::rename(displaced, root.path()).expect("restore root");
        assert!(matches!(result, Err(FilesystemError::Unverifiable(_))));
    }
}
