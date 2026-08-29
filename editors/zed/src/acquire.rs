//! Language Serverの自動取得のうち、Zedの実行環境に依存しない判断をまとめます。
//!
//! 版の選択、platformとtargetの対応、asset名の組み立ては、この場所だけで決めます。
use serde::Deserialize;

const TAG_PREFIX: &str = "v";

/// 取得済みのLanguage Serverです。
pub struct AcquiredServer {
    pub executable: String,
}

#[derive(Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

/// 選んだLanguage Serverのreleaseです。
pub struct SelectedRelease {
    pub version: String,
    pub assets: Vec<ReleaseAsset>,
}

impl SelectedRelease {
    pub fn asset(&self, name: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

/// 公開済みのLanguage Server releaseから最新版を選びます。
///
/// `latest_github_release`ではstable SemVerの検証ができないため使いません。
pub fn latest_lsp_release(body: &str) -> Result<SelectedRelease, String> {
    let releases: Vec<GithubRelease> = serde_json::from_str(body)
        .map_err(|error| format!("GitHubのrelease一覧を解釈できません：{error}"))?;
    let mut newest: Option<(Version, SelectedRelease)> = None;
    for release in releases {
        if release.draft || release.prerelease {
            continue;
        }
        let Some(version) = release.tag_name.strip_prefix(TAG_PREFIX) else {
            continue;
        };
        let Some(parsed) = Version::parse(version) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(current, _)| parsed > *current) {
            newest = Some((
                parsed,
                SelectedRelease {
                    version: version.to_owned(),
                    assets: release.assets,
                },
            ));
        }
    }
    newest
        .map(|(_, release)| release)
        .ok_or_else(|| "公開済みのAdocWeave Language Server releaseが見つかりません".to_owned())
}

/// 配布しているtargetと、Zedが報告するplatformの対応です。
///
/// Intel macOSとWindows ARM64向けのnative archiveは配布していません。
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
                "この環境向けのAdocWeave Language Serverは配布していません。対応はlinuxのx86_64とaarch64、macOSのaarch64、Windowsのx86_64です"
                    .to_owned(),
            );
        }
    };
    Ok(triple.to_owned())
}

pub fn asset_name(target: &str) -> String {
    format!("adocweave-{target}.zip")
}

/// 版の比較に使う、stable SemVerの三要素です。
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Version(u64, u64, u64);

impl Version {
    fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self(major, minor, patch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zed_extension_api::{Architecture, Os};

    fn release(tag: &str, assets: &[&str]) -> String {
        let assets = assets
            .iter()
            .map(|name| {
                format!(
                    r#"{{"name":"{name}","browser_download_url":"https://example.test/{name}"}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"assets":[{assets}]}}"#)
    }

    #[test]
    fn selects_the_newest_language_server_release() {
        let body = format!(
            "[{},{},{}]",
            release("v0.46.2", &["adocweave-x86_64-unknown-linux-musl.zip"]),
            release("adocweave-wasm/v0.48.0", &["adocweave-wasm-0.48.0.tgz"]),
            release("v0.47.0", &["adocweave-x86_64-unknown-linux-musl.zip"]),
        );

        let selected = latest_lsp_release(&body).expect("release");
        assert_eq!(selected.version, "0.47.0");
    }

    #[test]
    fn ignores_other_products_drafts_and_prereleases() {
        let body = format!(
            r#"[{},{{"tag_name":"v9.9.9","draft":true,"prerelease":false,"assets":[]}},{{"tag_name":"v8.8.8","draft":false,"prerelease":true,"assets":[]}}]"#,
            release("v0.47.0", &["adocweave-x86_64-unknown-linux-musl.zip"]),
        );

        assert_eq!(
            latest_lsp_release(&body).expect("release").version,
            "0.47.0"
        );
        assert!(latest_lsp_release(r#"[{"tag_name":"release-0.47.0"}]"#).is_err());
    }

    #[test]
    fn resolves_the_asset_for_the_selected_release() {
        let body = format!(
            "[{}]",
            release(
                "v0.47.0",
                &["adocweave-x86_64-unknown-linux-musl.zip", "sha256.sum"],
            )
        );
        let selected = latest_lsp_release(&body).expect("release");

        let asset = selected
            .asset(&asset_name("x86_64-unknown-linux-musl"))
            .expect("asset");
        assert_eq!(
            asset.browser_download_url,
            "https://example.test/adocweave-x86_64-unknown-linux-musl.zip"
        );
        assert!(selected
            .asset(&asset_name("aarch64-apple-darwin"))
            .is_none());
    }

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
        // Intel macOSとWindows ARM64は配布していない。
        assert!(target_triple(Os::Mac, Architecture::X8664).is_err());
        assert!(target_triple(Os::Windows, Architecture::Aarch64).is_err());
    }

    #[test]
    fn rejects_versions_that_are_not_stable_semver() {
        assert!(Version::parse("0.47.0").is_some());
        assert!(Version::parse("0.47").is_none());
        assert!(Version::parse("0.47.0.1").is_none());
        assert!(Version::parse("0.47.0-rc.1").is_none());
    }
}
