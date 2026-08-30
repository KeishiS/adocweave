use zed_extension_api as zed;

mod acquire;

use acquire::{asset_name, target_triple, AcquiredServer};

const SERVER_NAME: &str = "adocweave";
const SERVER_EXECUTABLE: &str = "adocweave";
const REPOSITORY: &str = "KeishiS/adocweave";
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
        // Always prefer a user-managed executable and use automatic acquisition last.
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
        let executable = match zed::latest_github_release(
            REPOSITORY,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        ) {
            Ok(release) => {
                let expected_asset = asset_name(&target);
                let asset = release
                    .assets
                    .iter()
                    .find(|asset| asset.name == expected_asset)
                    .ok_or_else(|| format!(
                        "AdocWeave release {} does not include an archive for {target}. See {DOCUMENTATION}",
                        release.version
                    ))?;
                let directory = version_directory(&release.version, &target);
                let executable = executable_path(&directory, os);
                if !is_file(&executable) {
                    zed::set_language_server_installation_status(
                        id,
                        &zed::LanguageServerInstallationStatus::Downloading,
                    );
                    // The Zed extension API cannot verify checksums, so integrity relies on TLS.
                    zed::download_file(
                        &asset.download_url,
                        &directory,
                        zed::DownloadedFileType::Zip,
                    )
                    .map_err(|error| {
                        format!(
                            "Could not download the AdocWeave Language Server: {error}. See {DOCUMENTATION}"
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
            // A downloaded release remains usable when GitHub is temporarily unavailable.
            Err(error) => downloaded_executable(os, &target).ok_or(error)?,
        };
        Ok(executable)
    }
}

fn version_directory(version: &str, target: &str) -> String {
    format!("{SERVER_EXECUTABLE}-{version}-{target}")
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

/// Returns one downloaded version whose executable still exists.
fn downloaded_executable(os: zed::Os, target: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for entry in std::fs::read_dir(".").ok()?.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&format!("{SERVER_EXECUTABLE}-"))
            || !name.ends_with(&format!("-{target}"))
        {
            continue;
        }
        let executable = executable_path(name, os);
        if is_file(&executable) && found.as_ref().is_none_or(|current| *current < executable) {
            found = Some(executable);
        }
    }
    found
}

/// Keeps only the selected download so updates do not grow the work directory indefinitely.
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
        args: vec!["lsp".to_owned()],
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
    fn server_command_uses_lsp_subcommand_and_preserves_shell_environment() {
        let env = vec![("PATH".to_owned(), "/usr/bin".to_owned())];
        let command = server_command("/usr/bin/adocweave".to_owned(), env.clone());

        assert_eq!(command.command, "/usr/bin/adocweave");
        assert_eq!(command.args, ["lsp"]);
        assert_eq!(command.env, env);
    }

    #[test]
    fn acquired_executable_path_contains_version_and_target() {
        let directory = version_directory("0.47.0", "x86_64-unknown-linux-musl");

        assert_eq!(directory, "adocweave-0.47.0-x86_64-unknown-linux-musl");
        assert_eq!(
            executable_path(&directory, zed::Os::Linux),
            format!("{directory}/adocweave")
        );
        assert_eq!(
            executable_path(&directory, zed::Os::Windows),
            format!("{directory}/adocweave.exe")
        );
    }

    #[test]
    fn accepts_only_host_absolute_paths() {
        assert!(is_absolute_path("/opt/adocweave", zed::Os::Linux));
        assert!(is_absolute_path("/opt/adocweave", zed::Os::Mac));
        assert!(is_absolute_path(
            r"C:\Tools\adocweave.exe",
            zed::Os::Windows
        ));
        assert!(is_absolute_path(
            r"\\server\tools\adocweave.exe",
            zed::Os::Windows
        ));
        assert!(!is_absolute_path("bin/adocweave", zed::Os::Linux));
        assert!(!is_absolute_path(r".\adocweave.exe", zed::Os::Windows));
        assert!(!is_absolute_path(r"\\server", zed::Os::Windows));
    }

    #[test]
    fn configured_path_ignores_arguments_and_environment() {
        let settings = zed::settings::LspSettings {
            binary: Some(zed::settings::CommandSettings {
                path: Some("/opt/adocweave".to_owned()),
                arguments: Some(vec!["--unexpected".to_owned()]),
                env: Some(HashMap::from([("UNEXPECTED".to_owned(), "1".to_owned())])),
            }),
            ..Default::default()
        };

        assert_eq!(
            configured_server_path(settings, zed::Os::Linux),
            Ok(Some("/opt/adocweave".to_owned()))
        );
        assert!(configured_server_path(
            zed::settings::LspSettings {
                binary: Some(zed::settings::CommandSettings {
                    path: Some("relative/adocweave".to_owned()),
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
