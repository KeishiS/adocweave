use zed_extension_api as zed;

mod acquire;

use acquire::{asset_name, latest_lsp_release, target_triple, AcquiredServer};

const SERVER_NAME: &str = "adocweave";
const SERVER_EXECUTABLE: &str = "adocweave-lsp";
const RELEASES_URL: &str = "https://api.github.com/repos/KeishiS/adocweave/releases?per_page=100";
const DOCUMENTATION: &str =
    "https://github.com/KeishiS/adocweave/blob/main/docs/user-guide/release-installation.adoc";

struct AdocWeaveExtension {
    acquired: Option<AcquiredServer>,
}

impl zed::Extension for AdocWeaveExtension {
    fn new() -> Self {
        Self { acquired: None }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let settings = zed::settings::LspSettings::for_worktree(SERVER_NAME, worktree)?;
        // 利用者が導入した実行ファイルを常に優先し、自動取得は最後の手段にする。
        let command =
            if let Some(path) = configured_server_path(settings, zed::current_platform().0)? {
                path
            } else if let Some(command) = worktree.which(SERVER_EXECUTABLE) {
                command
            } else {
                self.acquire_server(language_server_id)?
            };
        Ok(server_command(command, worktree.shell_env()))
    }
}

impl AdocWeaveExtension {
    fn acquire_server(&mut self, id: &zed::LanguageServerId) -> Result<String, String> {
        if let Some(acquired) = &self.acquired {
            if is_file(&acquired.executable) {
                return Ok(acquired.executable.clone());
            }
        }
        let (os, architecture) = zed::current_platform();
        let target = target_triple(os, architecture)?;

        zed::set_language_server_installation_status(
            id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let executable = match fetch_releases().and_then(|body| latest_lsp_release(&body)) {
            Ok(release) => {
                let asset = release.asset(&asset_name(&target)).ok_or_else(|| {
                    format!(
                        "AdocWeave Language Server {} に {target} の成果物がありません。導入手順は {DOCUMENTATION} を参照してください",
                        release.version
                    )
                })?;
                let directory = version_directory(&release.version);
                let executable = executable_path(&directory, os);
                if !is_file(&executable) {
                    zed::set_language_server_installation_status(
                        id,
                        &zed::LanguageServerInstallationStatus::Downloading,
                    );
                    // Zedの拡張APIにはchecksumを検証する手段がない。完全性はTLSだけに依存する。
                    zed::download_file(
                        &asset.browser_download_url,
                        &directory,
                        zed::DownloadedFileType::Zip,
                    )
                    .map_err(|error| {
                        format!(
                            "AdocWeave Language Serverを取得できませんでした：{error}。導入手順は {DOCUMENTATION} を参照してください"
                        )
                    })?;
                    zed::make_file_executable(&executable)?;
                }
                remove_other_versions(&directory);
                self.acquired = Some(AcquiredServer {
                    executable: executable.clone(),
                });
                executable
            }
            // 取得済みの版があれば、GitHubへ到達できなくても起動できる。
            Err(error) => downloaded_executable(os).ok_or(error)?,
        };
        Ok(executable)
    }
}

fn fetch_releases() -> Result<String, String> {
    let request = zed::http_client::HttpRequest::builder()
        .method(zed::http_client::HttpMethod::Get)
        .url(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "adocweave-zed-extension")
        .build()?;
    let response = request.fetch().map_err(|error| {
        format!(
            "GitHubのrelease一覧を取得できませんでした：{error}。導入手順は {DOCUMENTATION} を参照してください"
        )
    })?;
    String::from_utf8(response.body).map_err(|_| "GitHubの応答をUTF-8として読めません".to_owned())
}

fn version_directory(version: &str) -> String {
    format!("{SERVER_EXECUTABLE}-{version}")
}

fn executable_path(directory: &str, os: zed::Os) -> String {
    let suffix = match os {
        zed::Os::Windows => ".exe",
        zed::Os::Linux | zed::Os::Mac => "",
    };
    format!("{directory}/{SERVER_EXECUTABLE}{suffix}")
}

fn is_file(path: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|stat| stat.is_file())
}

/// 取得済みの版のうち、実行ファイルが残っているものを一つ返します。
fn downloaded_executable(os: zed::Os) -> Option<String> {
    let mut found: Option<String> = None;
    for entry in std::fs::read_dir(".").ok()?.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&format!("{SERVER_EXECUTABLE}-")) {
            continue;
        }
        let executable = executable_path(name, os);
        if is_file(&executable) && found.as_ref().is_none_or(|current| *current < executable) {
            found = Some(executable);
        }
    }
    found
}

/// 取得した版だけを残します。消さないと更新のたびに作業ディレクトリが増え続けます。
fn remove_other_versions(keep: &str) {
    let Ok(entries) = std::fs::read_dir(".") else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(&format!("{SERVER_EXECUTABLE}-")) && name != keep {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn configured_server_path(
    settings: zed::settings::LspSettings,
    os: zed::Os,
) -> Result<Option<String>, String> {
    let Some(path) = settings.binary.and_then(|binary| binary.path) else {
        return Ok(None);
    };
    if !is_absolute_path(&path, os) {
        return Err(format!(
            "lsp.{SERVER_NAME}.binary.path must be an absolute path"
        ));
    }
    Ok(Some(path))
}

fn server_command(command: String, env: Vec<(String, String)>) -> zed::Command {
    zed::Command {
        command,
        args: Vec::new(),
        env,
    }
}

fn is_absolute_path(path: &str, os: zed::Os) -> bool {
    match os {
        zed::Os::Windows => {
            let bytes = path.as_bytes();
            (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'/' | b'\\'))
                || windows_unc_path(bytes)
        }
        zed::Os::Linux | zed::Os::Mac => path.starts_with('/'),
    }
}

fn windows_unc_path(path: &[u8]) -> bool {
    let Some(rest) = path
        .strip_prefix(b"\\\\")
        .or_else(|| path.strip_prefix(b"//"))
    else {
        return false;
    };
    let mut components = rest.split(|byte| matches!(byte, b'/' | b'\\'));
    components.next().is_some_and(|server| !server.is_empty())
        && components.next().is_some_and(|share| !share.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn server_command_has_no_arguments_and_preserves_shell_environment() {
        let env = vec![("PATH".to_owned(), "/usr/bin".to_owned())];
        let command = server_command("/usr/bin/adocweave-lsp".to_owned(), env.clone());

        assert_eq!(command.command, "/usr/bin/adocweave-lsp");
        assert!(command.args.is_empty());
        assert_eq!(command.env, env);
    }

    #[test]
    fn accepts_only_host_absolute_paths() {
        assert!(is_absolute_path("/opt/adocweave-lsp", zed::Os::Linux));
        assert!(is_absolute_path("/opt/adocweave-lsp", zed::Os::Mac));
        assert!(is_absolute_path(
            r"C:\Tools\adocweave-lsp.exe",
            zed::Os::Windows
        ));
        assert!(is_absolute_path(
            r"\\server\tools\adocweave-lsp.exe",
            zed::Os::Windows
        ));
        assert!(!is_absolute_path("bin/adocweave-lsp", zed::Os::Linux));
        assert!(!is_absolute_path(r".\adocweave-lsp.exe", zed::Os::Windows));
        assert!(!is_absolute_path(r"\\server", zed::Os::Windows));
    }

    #[test]
    fn configured_path_ignores_arguments_and_environment() {
        let settings = zed::settings::LspSettings {
            binary: Some(zed::settings::CommandSettings {
                path: Some("/opt/adocweave-lsp".to_owned()),
                arguments: Some(vec!["--unexpected".to_owned()]),
                env: Some(HashMap::from([("UNEXPECTED".to_owned(), "1".to_owned())])),
            }),
            ..Default::default()
        };

        assert_eq!(
            configured_server_path(settings, zed::Os::Linux),
            Ok(Some("/opt/adocweave-lsp".to_owned()))
        );
        assert!(configured_server_path(
            zed::settings::LspSettings {
                binary: Some(zed::settings::CommandSettings {
                    path: Some("relative/adocweave-lsp".to_owned()),
                    arguments: None,
                    env: None,
                }),
                ..Default::default()
            },
            zed::Os::Linux,
        )
        .is_err());
    }
}

zed::register_extension!(AdocWeaveExtension);
