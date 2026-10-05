//! Dedicated generated directories shared by saving and static serving.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use adocweave_core::CancellationCheck;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::filesystem::RootAuthority;
use crate::{ProjectLimits, ProjectResourceError};

const MANIFEST: &str = ".adocweave-manifest.json";

/// Finite content types supported by the generated slide bundle.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BundleMediaType {
    Html,
    Css,
    JavaScript,
    Json,
    Text,
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
    Woff2,
}

impl BundleMediaType {
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Html => "text/html; charset=utf-8",
            Self::Css => "text/css; charset=utf-8",
            Self::JavaScript => "text/javascript; charset=utf-8",
            Self::Json => "application/json",
            Self::Text => "text/plain; charset=utf-8",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Svg => "image/svg+xml",
            Self::Woff2 => "font/woff2",
        }
    }

    fn matches_path(self, path: &str) -> bool {
        let extension = Path::new(path)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        match self {
            Self::Html => extension == "html",
            Self::Css => extension == "css",
            Self::JavaScript => extension == "js",
            Self::Json => extension == "json",
            Self::Text => extension == "txt",
            Self::Png => extension == "png",
            Self::Jpeg => matches!(extension, "jpg" | "jpeg"),
            Self::Gif => extension == "gif",
            Self::Webp => extension == "webp",
            Self::Svg => extension == "svg",
            Self::Woff2 => extension == "woff2",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleFile {
    pub path: String,
    pub media_type: BundleMediaType,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleManifestFile {
    pub path: String,
    pub media_type: BundleMediaType,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub generator: String,
    pub files: Vec<BundleManifestFile>,
}

/// A complete immutable bundle, validated before live output is adopted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleSnapshot {
    manifest: BundleManifest,
    files: BTreeMap<String, BundleFile>,
}

impl BundleSnapshot {
    pub fn from_files(
        files: Vec<BundleFile>,
        limits: ProjectLimits,
        cancellation: &dyn CancellationCheck,
    ) -> Result<Self, BundleError> {
        cancelled(cancellation)?;
        let (manifest, _) = prepare_manifest(&files, limits, cancellation)?;
        Ok(Self {
            manifest,
            files: files
                .into_iter()
                .map(|file| (file.path.clone(), file))
                .collect(),
        })
    }

    /// Copies only verified manifest files; later filesystem edits cannot
    /// partially change an already adopted HTTP generation.
    pub fn from_reader(
        reader: &ManagedBundleReader,
        cancellation: &dyn CancellationCheck,
    ) -> Result<Self, BundleError> {
        let mut files = BTreeMap::new();
        for entry in &reader.manifest.files {
            cancelled(cancellation)?;
            let (media_type, bytes) = reader.read_file(&entry.path)?;
            files.insert(
                entry.path.clone(),
                BundleFile {
                    path: entry.path.clone(),
                    media_type,
                    bytes,
                },
            );
        }
        Ok(Self {
            manifest: reader.manifest.clone(),
            files,
        })
    }

    pub fn manifest(&self) -> &BundleManifest {
        &self.manifest
    }

    /// Exact allowlist lookup; unlisted, encoded or traversing paths never
    /// select files from the ambient filesystem.
    pub fn file(&self, path: &str) -> Option<&BundleFile> {
        self.files.get(path)
    }
}

#[derive(Debug)]
pub enum BundleError {
    Invalid(String),
    Resource(ProjectResourceError),
    Io { path: PathBuf, message: String },
    Cancelled,
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "invalid managed bundle: {message}"),
            Self::Resource(error) => error.fmt(f),
            Self::Io { path, message } => write!(
                f,
                "cannot update managed bundle {}: {message}; keep this directory and regenerate into a new or empty output directory",
                path.to_string_lossy().escape_debug()
            ),
            Self::Cancelled => f.write_str("managed bundle operation was cancelled"),
        }
    }
}
impl std::error::Error for BundleError {}
impl From<crate::filesystem::FilesystemError> for BundleError {
    fn from(error: crate::filesystem::FilesystemError) -> Self {
        Self::Resource(ProjectResourceError::from_filesystem(error))
    }
}

fn io(path: &Path, error: std::io::Error) -> BundleError {
    BundleError::Io {
        path: path.to_owned(),
        message: error.to_string(),
    }
}
fn retained_error(cause: impl fmt::Display) -> BundleError {
    BundleError::Invalid(format!(
        "{cause}; keep this directory and regenerate into a new or empty output directory"
    ))
}
fn cancelled(cancellation: &dyn CancellationCheck) -> Result<(), BundleError> {
    if cancellation.is_cancelled() {
        Err(BundleError::Cancelled)
    } else {
        Ok(())
    }
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn relative_path(path: &str) -> Result<&Path, BundleError> {
    if path.is_empty()
        || !path.is_ascii()
        || !path
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || b"._-/".contains(&value))
    {
        return Err(BundleError::Invalid(
            "file paths must be plain relative ASCII paths".to_owned(),
        ));
    }
    let relative = Path::new(path);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
        || relative.is_absolute()
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(BundleError::Invalid(
            "absolute, empty and parent path components are forbidden".to_owned(),
        ));
    }
    Ok(relative)
}

fn validate_manifest(manifest: &BundleManifest, limits: ProjectLimits) -> Result<(), BundleError> {
    if manifest.schema_version != 1
        || manifest.generator != "adocweave-slides"
        || manifest.files.is_empty()
        || manifest.files.len() >= limits.resources.max_files
    {
        return Err(BundleError::Invalid(
            "unsupported or oversized manifest".to_owned(),
        ));
    }
    let mut paths = BTreeSet::new();
    let mut total = 0u64;
    for file in &manifest.files {
        relative_path(&file.path)?;
        if file.path == MANIFEST
            || !paths.insert(&file.path)
            || !file.media_type.matches_path(&file.path)
            || file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
        {
            return Err(BundleError::Invalid(
                "invalid, duplicate or incompatible manifest file".to_owned(),
            ));
        }
        total = total
            .checked_add(file.size_bytes)
            .ok_or_else(|| BundleError::Invalid("bundle size overflow".to_owned()))?;
        if file.size_bytes > limits.resources.max_resource_bytes
            || total > u64::from(limits.max_output_bytes).min(limits.resources.max_total_bytes)
        {
            return Err(BundleError::Invalid(
                "bundle exceeds configured byte limits".to_owned(),
            ));
        }
    }
    if !paths.contains(&"index.html".to_owned()) {
        return Err(BundleError::Invalid(
            "bundle must contain index.html".to_owned(),
        ));
    }
    if expected_directories(manifest)
        .iter()
        .any(|path| paths.iter().any(|file| Path::new(file.as_str()) == path))
    {
        return Err(BundleError::Invalid(
            "a file path also names a directory".to_owned(),
        ));
    }
    Ok(())
}

fn expected_directories(manifest: &BundleManifest) -> BTreeSet<PathBuf> {
    manifest
        .files
        .iter()
        .flat_map(|file| {
            Path::new(&file.path)
                .ancestors()
                .skip(1)
                .filter(|path| !path.as_os_str().is_empty())
                .map(Path::to_owned)
        })
        .collect()
}

fn entries(
    authority: &RootAuthority,
    limits: ProjectLimits,
    cancellation: &dyn CancellationCheck,
) -> Result<(BTreeSet<String>, BTreeSet<PathBuf>), BundleError> {
    let mut pending = vec![PathBuf::new()];
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut count = 0u64;
    while let Some(relative) = pending.pop() {
        cancelled(cancellation)?;
        let logical = authority.root().join(&relative);
        let (_directory, operational) = authority.operation_directory(&logical)?;
        for entry in fs::read_dir(&operational).map_err(|error| io(&logical, error))? {
            let entry = entry.map_err(|error| io(&logical, error))?;
            count += 1;
            if count > limits.max_directory_entries {
                return Err(BundleError::Invalid(
                    "directory inspection limit exceeded".to_owned(),
                ));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| BundleError::Invalid("non-UTF-8 output file name".to_owned()))?;
            let path = relative.join(name);
            let kind = entry.file_type().map_err(|error| io(&logical, error))?;
            if kind.is_symlink() {
                return Err(retained_error(format!(
                    "symbolic link in managed bundle: {}",
                    path.to_string_lossy().escape_debug()
                )));
            }
            if kind.is_dir() {
                directories.insert(path.clone());
                pending.push(path);
            } else if kind.is_file() {
                files.insert(
                    path.to_str()
                        .expect("UTF-8 path components")
                        .replace('\\', "/"),
                );
            } else {
                return Err(BundleError::Invalid(
                    "only regular files and directories are allowed".to_owned(),
                ));
            }
        }
    }
    Ok((files, directories))
}

fn verified_files(
    authority: &RootAuthority,
    manifest: &BundleManifest,
    limits: ProjectLimits,
    cancellation: &dyn CancellationCheck,
) -> Result<BTreeMap<String, Vec<u8>>, BundleError> {
    validate_manifest(manifest, limits)?;
    let (files, directories) = entries(authority, limits, cancellation)?;
    let expected = manifest
        .files
        .iter()
        .map(|file| file.path.clone())
        .chain([MANIFEST.to_owned()])
        .collect::<BTreeSet<_>>();
    let expected_directories = expected_directories(manifest);
    for (kind, path) in [
        (
            "unknown file",
            files.difference(&expected).next().map(PathBuf::from),
        ),
        (
            "missing managed file",
            expected.difference(&files).next().map(PathBuf::from),
        ),
        (
            "unknown directory",
            directories
                .difference(&expected_directories)
                .next()
                .cloned(),
        ),
        (
            "missing managed directory",
            expected_directories
                .difference(&directories)
                .next()
                .cloned(),
        ),
    ] {
        if let Some(path) = path {
            return Err(retained_error(format!(
                "{kind}: {}",
                path.to_string_lossy().escape_debug()
            )));
        }
    }
    verified_manifest_contents(authority, manifest, cancellation)
}

fn verified_manifest_contents(
    authority: &RootAuthority,
    manifest: &BundleManifest,
    cancellation: &dyn CancellationCheck,
) -> Result<BTreeMap<String, Vec<u8>>, BundleError> {
    let mut contents = BTreeMap::new();
    for file in &manifest.files {
        cancelled(cancellation)?;
        let bytes = authority
            .read_binary(&authority.root().join(&file.path), file.size_bytes)
            .map_err(|error| match error {
                crate::filesystem::FilesystemError::ResourceTooLarge(_) => {
                    retained_error(format!("managed file was edited: {}", file.path))
                }
                error => error.into(),
            })?;
        if bytes.len() as u64 != file.size_bytes || digest(&bytes) != file.sha256 {
            return Err(retained_error(format!(
                "managed file was edited: {}",
                file.path
            )));
        }
        contents.insert(file.path.clone(), bytes);
    }
    Ok(contents)
}

/// A validated allowlist and retained directory authority for static serving.
#[derive(Debug)]
pub struct ManagedBundleReader {
    authority: RootAuthority,
    manifest: BundleManifest,
    limits: ProjectLimits,
}

impl ManagedBundleReader {
    pub fn manifest(&self) -> &BundleManifest {
        &self.manifest
    }
    /// Reads only a listed file and checks its digest again before returning it.
    pub fn read_file(&self, path: &str) -> Result<(BundleMediaType, Vec<u8>), BundleError> {
        relative_path(path)?;
        let file = self
            .manifest
            .files
            .iter()
            .find(|file| file.path == path)
            .ok_or_else(|| {
                BundleError::Invalid("file is outside the manifest allowlist".to_owned())
            })?;
        let bytes = self.authority.read_binary(
            &self.authority.root().join(path),
            file.size_bytes
                .min(self.limits.resources.max_resource_bytes),
        )?;
        if bytes.len() as u64 != file.size_bytes || digest(&bytes) != file.sha256 {
            return Err(retained_error(format!(
                "managed file changed after validation: {}",
                file.path
            )));
        }
        Ok((file.media_type, bytes))
    }
}

fn read_manifest(
    authority: &RootAuthority,
    limits: ProjectLimits,
) -> Result<(BundleManifest, Vec<u8>), BundleError> {
    let bytes = authority
        .read_binary(
            &authority.root().join(MANIFEST),
            limits
                .resources
                .max_resource_bytes
                .min(u64::from(limits.max_output_bytes)),
        )
        .map_err(|error| match error {
            crate::filesystem::FilesystemError::Missing(_) => retained_error(format!(
                "output is not a managed bundle: missing {MANIFEST}"
            )),
            error => error.into(),
        })?;
    let manifest = serde_json::from_slice(&bytes)
        .map_err(|_| retained_error(format!("malformed manifest: {MANIFEST}")))?;
    validate_manifest(&manifest, limits)?;
    let contents: u64 = manifest.files.iter().map(|file| file.size_bytes).sum();
    if contents + bytes.len() as u64
        > u64::from(limits.max_output_bytes).min(limits.resources.max_total_bytes)
    {
        return Err(BundleError::Invalid(
            "bundle including its manifest exceeds output limits".to_owned(),
        ));
    }
    Ok((manifest, bytes))
}

/// Opens an existing bundle without acquiring access to unlisted files.
pub fn open_managed_bundle(
    directory: &Path,
    limits: ProjectLimits,
    cancellation: &dyn CancellationCheck,
) -> Result<ManagedBundleReader, BundleError> {
    reject_directory_path(directory, &[])?;
    let authority = open_directory(directory, false)?;
    let (manifest, _) = read_manifest(&authority, limits)?;
    verified_manifest_contents(&authority, &manifest, cancellation)?;
    Ok(ManagedBundleReader {
        authority,
        manifest,
        limits,
    })
}

fn reject_directory_path(directory: &Path, protected: &[PathBuf]) -> Result<(), BundleError> {
    if !directory.is_absolute()
        || directory.parent().is_none()
        || directory
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || protected.iter().any(|path| path == directory)
    {
        return Err(BundleError::Invalid(
            "output must be a dedicated directory outside source directories".to_owned(),
        ));
    }
    // Reject symbolic links before establishing explicit output authority.
    let mut path = PathBuf::new();
    for component in directory.components() {
        path.push(component);
        // A Windows prefix alone is not a directory; inspect its following root.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(BundleError::Invalid(format!(
                    "symbolic link in output path: {}; choose a path without symbolic links",
                    path.to_string_lossy().escape_debug()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(BundleError::Invalid(format!(
                    "output path component is not a directory: {}",
                    path.to_string_lossy().escape_debug()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(io(&path, error)),
        }
    }
    Ok(())
}

fn open_directory(directory: &Path, create: bool) -> Result<RootAuthority, BundleError> {
    let mut ancestor = directory;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| BundleError::Invalid("output has no existing ancestor".to_owned()))?;
    }
    let authority = RootAuthority::new(ancestor)?;
    if create {
        Ok(authority.create_directory_tree(directory)?)
    } else {
        Ok(authority.derive_confined_directory(directory)?)
    }
}

fn write_file(
    authority: &RootAuthority,
    relative: &str,
    original: Option<&[u8]>,
    replacement: &[u8],
) -> Result<(), BundleError> {
    let path = authority.root().join(relative_path(relative)?);
    if let Some(original) = original {
        if !authority.replace_candidate_after_recheck(&path, original, replacement)? {
            return Err(retained_error(format!(
                "managed file changed during generation: {relative}"
            )));
        }
    } else {
        let parent = path.parent().expect("relative file has a parent");
        let (_directory, operational) = authority.operation_directory(parent)?;
        let target = operational.join(path.file_name().expect("relative file has a name"));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(|error| io(&path, error))?;
        file.write_all(replacement)
            .and_then(|()| file.sync_all())
            .map_err(|error| io(&path, error))?;
    }
    Ok(())
}

// Validate the same finite file contract before saving or adopting a snapshot.
fn prepare_manifest(
    files: &[BundleFile],
    limits: ProjectLimits,
    cancellation: &dyn CancellationCheck,
) -> Result<(BundleManifest, Vec<u8>), BundleError> {
    let mut manifest = BundleManifest {
        schema_version: 1,
        generator: "adocweave-slides".to_owned(),
        files: files
            .iter()
            .map(|file| {
                cancelled(cancellation)?;
                Ok(BundleManifestFile {
                    path: file.path.clone(),
                    media_type: file.media_type,
                    size_bytes: file.bytes.len() as u64,
                    sha256: digest(&file.bytes),
                })
            })
            .collect::<Result<Vec<_>, BundleError>>()?,
    };
    manifest.files.sort_by(|a, b| a.path.cmp(&b.path));
    validate_manifest(&manifest, limits)?;
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|_| BundleError::Invalid("cannot serialize manifest".to_owned()))?;
    manifest_bytes.push(b'\n');
    if manifest_bytes.len() as u64 > limits.resources.max_resource_bytes
        || manifest_bytes.len() as u64
            + files
                .iter()
                .map(|file| file.bytes.len() as u64)
                .sum::<u64>()
            > u64::from(limits.max_output_bytes).min(limits.resources.max_total_bytes)
    {
        return Err(BundleError::Invalid(
            "bundle including its manifest exceeds output limits".to_owned(),
        ));
    }
    Ok((manifest, manifest_bytes))
}

/// Saves only into a new, empty, or fully verified generated directory.
///
/// Existing unknown or edited files are never overwritten. A failed update can
/// leave a partial bundle, but is never reported as successful or served as valid.
pub fn save_managed_bundle(
    directory: &Path,
    protected_source_directories: &[PathBuf],
    files: &[BundleFile],
    limits: ProjectLimits,
    cancellation: &dyn CancellationCheck,
) -> Result<BundleManifest, BundleError> {
    reject_directory_path(directory, protected_source_directories)?;
    let (manifest, manifest_bytes) = prepare_manifest(files, limits, cancellation)?;
    cancelled(cancellation)?;
    let authority = open_directory(directory, true)?;
    let (existing, directories) = entries(&authority, limits, cancellation)?;
    let (old, old_manifest, old_bytes) = if existing.is_empty() && directories.is_empty() {
        (None, None, BTreeMap::new())
    } else {
        let (old, raw) = read_manifest(&authority, limits)?;
        let contents = verified_files(&authority, &old, limits, cancellation)?;
        (Some(old), Some(raw), contents)
    };
    // Repeat the complete check immediately before the first mutation.
    reject_directory_path(directory, protected_source_directories)?;
    authority.verify_directory_namespace()?;
    if let Some(old) = &old {
        verified_files(&authority, old, limits, cancellation)?;
    } else if entries(&authority, limits, cancellation)? != (BTreeSet::new(), BTreeSet::new()) {
        return Err(BundleError::Invalid(
            "output changed before generation".to_owned(),
        ));
    }
    for relative in expected_directories(&manifest) {
        authority.create_directory_tree(&directory.join(relative))?;
    }
    for file in files {
        cancelled(cancellation)?;
        write_file(
            &authority,
            &file.path,
            old_bytes.get(&file.path).map(Vec::as_slice),
            &file.bytes,
        )?;
    }
    if let Some(old) = &old {
        let next = manifest
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<BTreeSet<_>>();
        for file in &old.files {
            if next.contains(file.path.as_str()) {
                continue;
            }
            cancelled(cancellation)?;
            let path = directory.join(&file.path);
            if !authority.candidate_contents_match(&path, &old_bytes[&file.path])? {
                return Err(retained_error(format!(
                    "stale managed file changed before removal: {}",
                    file.path
                )));
            }
            let (_parent, operational) =
                authority.operation_directory(path.parent().expect("managed file has a parent"))?;
            fs::remove_file(operational.join(path.file_name().expect("managed file has a name")))
                .map_err(|error| io(&path, error))?;
        }
        let next_dirs = expected_directories(&manifest);
        let mut stale = expected_directories(old)
            .difference(&next_dirs)
            .cloned()
            .collect::<Vec<_>>();
        stale.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for relative in stale {
            let path = directory.join(relative);
            let (_parent, operational) = authority
                .operation_directory(path.parent().expect("managed directory has a parent"))?;
            fs::remove_dir(
                operational.join(path.file_name().expect("managed directory has a name")),
            )
            .map_err(|error| io(&path, error))?;
        }
    }
    cancelled(cancellation)?;
    write_file(
        &authority,
        MANIFEST,
        old_manifest.as_deref(),
        &manifest_bytes,
    )?;
    verified_files(&authority, &manifest, limits, cancellation)?;
    reject_directory_path(directory, protected_source_directories)?;
    authority.verify_directory_namespace()?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use adocweave_core::{CancellationToken, NeverCancel};

    fn file(path: &str, bytes: &[u8]) -> BundleFile {
        BundleFile {
            path: path.to_owned(),
            media_type: if path.ends_with(".png") {
                BundleMediaType::Png
            } else {
                BundleMediaType::Html
            },
            bytes: bytes.to_vec(),
        }
    }
    fn save(directory: &Path, files: &[BundleFile]) -> Result<BundleManifest, BundleError> {
        save_managed_bundle(
            directory,
            &[],
            files,
            ProjectLimits::default(),
            &NeverCancel,
        )
    }
    fn root() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().canonicalize().unwrap();
        (directory, path)
    }

    #[test]
    fn presenter_to_public_removes_stale_assets_and_uses_the_same_serve_allowlist() {
        let (_root, path) = root();
        let output = path.join("dist/talk");
        save(
            &output,
            &[
                file("index.html", b"Presenter"),
                file("notes/private.png", b"secret asset"),
            ],
        )
        .unwrap();
        let manifest = save(&output, &[file("index.html", b"Public")]).unwrap();
        assert!(!output.join("notes").exists());
        assert_eq!(manifest.files.len(), 1);
        let reader = open_managed_bundle(&output, ProjectLimits::default(), &NeverCancel).unwrap();
        assert_eq!(
            reader.read_file("index.html").unwrap(),
            (BundleMediaType::Html, b"Public".to_vec())
        );
        for path in [
            "../outside.html",
            "/index.html",
            "notes/private.png",
            MANIFEST,
        ] {
            assert!(reader.read_file(path).is_err(), "{path}");
        }
        assert!(
            !fs::read_to_string(output.join(MANIFEST))
                .unwrap()
                .contains("private")
        );
    }

    #[test]
    fn serving_ignores_unlisted_files_but_saving_preserves_them() {
        let (_root, path) = root();
        let output = path.join("talk");
        save(&output, &[file("index.html", b"body")]).unwrap();
        fs::write(output.join(".DS_Store"), b"metadata").unwrap();
        fs::create_dir(output.join("private")).unwrap();
        fs::write(output.join("private/notes.txt"), b"private").unwrap();
        let reader = open_managed_bundle(&output, ProjectLimits::default(), &NeverCancel).unwrap();
        assert_eq!(reader.read_file("index.html").unwrap().1, b"body");
        let snapshot = BundleSnapshot::from_reader(&reader, &NeverCancel).unwrap();
        for path in [".DS_Store", "private/notes.txt"] {
            assert!(reader.read_file(path).is_err());
            assert!(snapshot.file(path).is_none());
        }
        assert!(save(&output, &[file("index.html", b"replacement")]).is_err());
        assert_eq!(fs::read(output.join("index.html")).unwrap(), b"body");
        assert_eq!(fs::read(output.join(".DS_Store")).unwrap(), b"metadata");
        assert_eq!(
            fs::read(output.join("private/notes.txt")).unwrap(),
            b"private"
        );
        fs::write(output.join("index.html"), b"edited").unwrap();
        assert!(open_managed_bundle(&output, ProjectLimits::default(), &NeverCancel).is_err());
    }

    #[test]
    fn adopted_snapshot_keeps_all_bytes_after_the_managed_directory_changes() {
        let (_root, path) = root();
        let output = path.join("talk");
        save(
            &output,
            &[
                file("index.html", b"Presenter"),
                file("notes/private.png", b"private"),
            ],
        )
        .unwrap();
        let reader = open_managed_bundle(&output, ProjectLimits::default(), &NeverCancel).unwrap();
        let snapshot = BundleSnapshot::from_reader(&reader, &NeverCancel).unwrap();
        save(&output, &[file("index.html", b"Public")]).unwrap();
        assert_eq!(snapshot.file("index.html").unwrap().bytes, b"Presenter");
        assert_eq!(
            snapshot.file("notes/private.png").unwrap().bytes,
            b"private"
        );
        assert!(reader.read_file("index.html").is_err());
        for path in ["../index.html", "/index.html", MANIFEST, "missing.png"] {
            assert!(snapshot.file(path).is_none());
        }
        let reader = open_managed_bundle(&output, ProjectLimits::default(), &NeverCancel).unwrap();
        let public = BundleSnapshot::from_reader(&reader, &NeverCancel).unwrap();
        assert_eq!(public.file("index.html").unwrap().bytes, b"Public");
        assert!(public.file("notes/private.png").is_none());
    }

    #[test]
    fn snapshots_reject_duplicate_paths_output_limits_and_cancellation() {
        assert!(
            BundleSnapshot::from_files(
                vec![file("index.html", b"a"), file("index.html", b"b")],
                ProjectLimits::default(),
                &NeverCancel
            )
            .is_err()
        );
        let limits = ProjectLimits {
            max_output_bytes: 20,
            ..Default::default()
        };
        assert!(
            BundleSnapshot::from_files(vec![file("index.html", b"a")], limits, &NeverCancel)
                .is_err()
        );
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            BundleSnapshot::from_files(
                vec![file("index.html", b"a")],
                ProjectLimits::default(),
                &cancellation
            ),
            Err(BundleError::Cancelled)
        ));
    }

    #[test]
    fn unknown_and_manually_edited_files_are_retained_without_any_update() {
        let (_root, path) = root();
        let output = path.join("talk");
        save(&output, &[file("index.html", b"original")]).unwrap();
        fs::write(output.join("manual.txt"), "manual").unwrap();
        let error = save(&output, &[file("index.html", b"replacement")]).unwrap_err();
        assert!(error.to_string().contains("unknown file: manual.txt"));
        assert!(error.to_string().contains("new or empty output directory"));
        assert_eq!(fs::read(output.join("index.html")).unwrap(), b"original");
        assert_eq!(fs::read(output.join("manual.txt")).unwrap(), b"manual");
        fs::remove_file(output.join("manual.txt")).unwrap();
        fs::write(output.join("index.html"), "manual edit").unwrap();
        let error = save(&output, &[file("index.html", b"replacement")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("managed file was edited: index.html")
        );
        assert_eq!(fs::read(output.join("index.html")).unwrap(), b"manual edit");
        assert!(open_managed_bundle(&output, ProjectLimits::default(), &NeverCancel).is_err());
    }

    #[test]
    fn nonempty_unmanaged_missing_and_metadata_files_have_specific_diagnostics() {
        let (_root, path) = root();
        let unmanaged = path.join("manual");
        fs::create_dir(&unmanaged).unwrap();
        fs::write(unmanaged.join("manual.txt"), b"keep").unwrap();
        let error = save(&unmanaged, &[file("index.html", b"public")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("output is not a managed bundle: missing")
        );
        assert!(error.to_string().contains(MANIFEST));
        assert_eq!(fs::read(unmanaged.join("manual.txt")).unwrap(), b"keep");
        assert!(!unmanaged.join("index.html").exists());

        let managed = path.join("talk");
        save(&managed, &[file("index.html", b"original")]).unwrap();
        fs::remove_file(managed.join("index.html")).unwrap();
        let error = save(&managed, &[file("index.html", b"replacement")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing managed file: index.html")
        );
        fs::write(managed.join("index.html"), b"original").unwrap();
        fs::write(managed.join(".DS_Store"), b"metadata").unwrap();
        let error = save(&managed, &[file("index.html", b"replacement")]).unwrap_err();
        assert!(error.to_string().contains("unknown file: .DS_Store"));
        assert_eq!(fs::read(managed.join(".DS_Store")).unwrap(), b"metadata");
        assert_eq!(fs::read(managed.join("index.html")).unwrap(), b"original");
    }

    #[test]
    fn unsafe_paths_and_overlapping_file_directories_are_rejected_before_creation() {
        let (_root, path) = root();
        for unsafe_path in [
            "../escape.html",
            "/escape.html",
            "C:/escape.html",
            "a/../escape.html",
            "a//escape.html",
            "a\\escape.html",
            "%2e%2e/escape.html",
        ] {
            let output = path.join("talk");
            assert!(
                save(
                    &output,
                    &[file("index.html", b"body"), file(unsafe_path, b"bad")]
                )
                .is_err()
            );
            assert!(!output.exists());
        }
        assert!(
            save(
                &path.join("talk"),
                &[file("index.html", b"first"), file("index.html", b"repeat")]
            )
            .is_err()
        );
        assert!(
            save(
                &path.join("talk"),
                &[
                    file("index.html", b"first"),
                    file("index.html/nested.html", b"nested")
                ]
            )
            .is_err()
        );
        assert!(
            save_managed_bundle(
                &path,
                std::slice::from_ref(&path),
                &[file("index.html", b"body")],
                ProjectLimits::default(),
                &NeverCancel
            )
            .is_err()
        );
        assert!(save(Path::new("/"), &[file("index.html", b"body")]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefixes_keep_roots_and_drive_relative_outputs_forbidden() {
        for directory in [
            r"C:\",
            r"\\?\C:\",
            r"\\server\share\",
            r"\\?\UNC\server\share\",
            r"C:talk",
            r"\talk",
        ] {
            assert!(
                matches!(
                    reject_directory_path(Path::new(directory), &[]),
                    Err(BundleError::Invalid(_))
                ),
                "{directory}"
            );
        }
        let (_root, path) = root();
        reject_directory_path(&path.join("talk"), &[]).unwrap();
    }

    #[test]
    fn cancellation_and_the_combined_size_limit_create_no_bundle() {
        let (_root, path) = root();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            save_managed_bundle(
                &path.join("cancelled"),
                &[],
                &[file("index.html", b"body")],
                ProjectLimits::default(),
                &cancellation
            ),
            Err(BundleError::Cancelled)
        ));
        assert!(!path.join("cancelled").exists());
        let limits = ProjectLimits {
            max_output_bytes: 20,
            ..Default::default()
        };
        assert!(
            save_managed_bundle(
                &path.join("limited"),
                &[],
                &[file("index.html", b"body")],
                limits,
                &NeverCancel
            )
            .is_err()
        );
        assert!(!path.join("limited/index.html").exists());
    }

    #[test]
    fn failed_stale_directory_cleanup_is_never_reported_as_public_success() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (_root, path) = root();
        let directory = path.join("talk");
        save(
            &directory,
            &[
                file("index.html", b"presenter"),
                file("notes/private.png", b"private"),
            ],
        )
        .unwrap();
        let old_manifest = fs::read(directory.join(MANIFEST)).unwrap();
        struct AddLateFile {
            directory: PathBuf,
            changed: AtomicBool,
        }
        impl CancellationCheck for AddLateFile {
            fn is_cancelled(&self) -> bool {
                if fs::read(self.directory.join("index.html")).is_ok_and(|bytes| bytes == b"public")
                    && !self.changed.swap(true, Ordering::SeqCst)
                {
                    fs::write(self.directory.join("notes/late-manual.txt"), b"manual").unwrap();
                }
                false
            }
        }
        let cancellation = AddLateFile {
            directory: directory.clone(),
            changed: AtomicBool::new(false),
        };
        let result = save_managed_bundle(
            &directory,
            &[],
            &[file("index.html", b"public")],
            ProjectLimits::default(),
            &cancellation,
        );
        assert!(matches!(result, Err(BundleError::Io { .. })));
        let message = result.unwrap_err().to_string();
        assert!(message.contains("notes"));
        assert!(message.contains("new or empty output directory"));
        assert_eq!(fs::read(directory.join(MANIFEST)).unwrap(), old_manifest);
        assert_eq!(
            fs::read(directory.join("notes/late-manual.txt")).unwrap(),
            b"manual"
        );
        assert!(open_managed_bundle(&directory, ProjectLimits::default(), &NeverCancel).is_err());

        let error = save(&directory, &[file("index.html", b"public")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unknown file: notes/late-manual.txt")
        );
        let retained = path.join("talk-retained");
        fs::rename(&directory, &retained).unwrap();
        save(&directory, &[file("index.html", b"public")]).unwrap();
        assert_eq!(fs::read(retained.join(MANIFEST)).unwrap(), old_manifest);
        assert_eq!(
            fs::read(retained.join("notes/late-manual.txt")).unwrap(),
            b"manual"
        );
        assert!(!directory.join("notes").exists());
        open_managed_bundle(&directory, ProjectLimits::default(), &NeverCancel).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_refused_at_the_output_directory_and_file() {
        use std::os::unix::fs::symlink;
        let (_root, path) = root();
        fs::create_dir(path.join("real")).unwrap();
        symlink(path.join("real"), path.join("alias")).unwrap();
        let error = save(&path.join("alias/talk"), &[file("index.html", b"body")]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&path.join("alias").to_string_lossy().to_string())
        );
        assert!(!path.join("real/talk").exists());
        save(&path.join("talk"), &[file("index.html", b"body")]).unwrap();
        fs::remove_file(path.join("talk/index.html")).unwrap();
        fs::write(path.join("outside.html"), "outside").unwrap();
        symlink(path.join("outside.html"), path.join("talk/index.html")).unwrap();
        assert!(
            open_managed_bundle(&path.join("talk"), ProjectLimits::default(), &NeverCancel)
                .is_err()
        );
        assert!(save(&path.join("talk"), &[file("index.html", b"replacement")]).is_err());
        assert_eq!(fs::read(path.join("outside.html")).unwrap(), b"outside");
        fs::remove_file(path.join("talk/index.html")).unwrap();
        fs::write(path.join("talk/copy.html"), b"body").unwrap();
        symlink("copy.html", path.join("talk/index.html")).unwrap();
        assert!(
            open_managed_bundle(&path.join("talk"), ProjectLimits::default(), &NeverCancel)
                .is_err()
        );
    }
}
