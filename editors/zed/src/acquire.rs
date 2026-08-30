//! Platform-specific decisions for automatic Language Server acquisition.

/// A downloaded Language Server executable.
pub struct AcquiredServer {
    pub executable: String,
}

/// Maps a Zed platform to a distributed native target.
///
/// Native archives are not available for Intel macOS or Windows ARM64.
pub fn target_triple(
    os: zed_extension_api::Os,
    arch: zed_extension_api::Architecture,
) -> Result<String, String> {
    use zed_extension_api::{Architecture, Os};
    let triple = match (os, arch) {
        (Os::Linux, Architecture::X8664) => "x86_64-unknown-linux-musl",
        (Os::Linux, Architecture::Aarch64) => "aarch64-unknown-linux-musl",
        (Os::Mac, Architecture::Aarch64) => "aarch64-apple-darwin",
        (Os::Windows, Architecture::X8664) => "x86_64-pc-windows-msvc",
        _ => {
            return Err(
                "AdocWeave is available for Linux x86_64 and ARM64, macOS ARM64, and Windows x86_64"
                    .to_owned(),
            );
        }
    };
    Ok(triple.to_owned())
}

pub fn asset_name(target: &str) -> String {
    format!("adocweave-{target}.zip")
}

#[cfg(test)]
mod tests {
    use super::*;
    use zed_extension_api::{Architecture, Os};

    #[test]
    fn maps_only_the_distributed_targets() {
        assert_eq!(
            target_triple(Os::Linux, Architecture::X8664).unwrap(),
            "x86_64-unknown-linux-musl"
        );
        assert_eq!(
            target_triple(Os::Linux, Architecture::Aarch64).unwrap(),
            "aarch64-unknown-linux-musl"
        );
        assert_eq!(
            target_triple(Os::Mac, Architecture::Aarch64).unwrap(),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            target_triple(Os::Windows, Architecture::X8664).unwrap(),
            "x86_64-pc-windows-msvc"
        );
        // Native archives are not distributed for Intel macOS or Windows ARM64.
        assert!(target_triple(Os::Mac, Architecture::X8664).is_err());
        assert!(target_triple(Os::Windows, Architecture::Aarch64).is_err());
    }

    #[test]
    fn uses_the_native_release_asset_name() {
        assert_eq!(
            asset_name("x86_64-unknown-linux-musl"),
            "adocweave-x86_64-unknown-linux-musl.zip"
        );
    }
}
