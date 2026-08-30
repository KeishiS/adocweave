use std::path::{Path, PathBuf};

use adocweave_project::ProjectResourceObservation;

/// Stable logical identity of one file observed by live preview.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Dependency {
    path: PathBuf,
    kind: DependencyKind,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum DependencyKind {
    Contents,
    ContentsNoSymlinks,
    Existence,
}

impl Dependency {
    pub(crate) fn contents(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: DependencyKind::Contents,
        }
    }

    pub(crate) fn contents_no_symlinks(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: DependencyKind::ContentsNoSymlinks,
        }
    }

    pub(crate) fn existence(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: DependencyKind::Existence,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) const fn kind(&self) -> DependencyKind {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Fingerprint {
    observation: ProjectResourceObservation,
}

impl Fingerprint {
    /// Captures content which was read through the dependency's authority.
    #[cfg(test)]
    pub(crate) fn from_loaded_bytes(bytes: &[u8]) -> Self {
        Self {
            observation: ProjectResourceObservation::from_bytes(bytes),
        }
    }

    #[cfg(test)]
    pub(crate) const fn present() -> Self {
        Self {
            observation: ProjectResourceObservation::present(),
        }
    }

    #[cfg(test)]
    pub(crate) const fn missing() -> Self {
        Self {
            observation: ProjectResourceObservation::missing(),
        }
    }

    pub(crate) fn from_observation(observation: ProjectResourceObservation) -> Self {
        Self { observation }
    }

    /// Captures a typed read failure without exposing filesystem paths as data.
    pub(crate) fn unavailable(_reason: &str) -> Self {
        Self {
            observation: ProjectResourceObservation::unavailable(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_fingerprints_detect_same_length_changes() {
        assert_ne!(
            Fingerprint::from_loaded_bytes(b"one"),
            Fingerprint::from_loaded_bytes(b"two")
        );
    }

    #[test]
    fn dependencies_use_their_absolute_path_as_identity() {
        assert_eq!(
            Dependency::contents(PathBuf::from("style.css")),
            Dependency::contents(PathBuf::from("style.css"))
        );
    }

    #[test]
    fn observation_kind_is_part_of_dependency_identity() {
        assert_ne!(
            Dependency::contents(PathBuf::from("asset.bin")),
            Dependency::existence(PathBuf::from("asset.bin"))
        );
    }

    #[test]
    fn unavailable_reasons_share_one_stable_fingerprint() {
        assert_eq!(
            Fingerprint::unavailable("missing"),
            Fingerprint::unavailable("missing")
        );
        assert_eq!(
            Fingerprint::unavailable("missing"),
            Fingerprint::unavailable("permission-denied")
        );
    }
}
