use zed_extension_api as zed;

const SERVER_NAME: &str = "adocweave";
const SERVER_EXECUTABLE: &str = "adocweave-lsp";

struct AdocWeaveExtension;

impl zed::Extension for AdocWeaveExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let settings = zed::settings::LspSettings::for_worktree(SERVER_NAME, worktree)?;
        let command = if let Some(path) = settings.binary.and_then(|binary| binary.path) {
            if !is_absolute_path(&path, zed::current_platform().0) {
                return Err(format!(
                    "lsp.{SERVER_NAME}.binary.path must be an absolute path"
                ));
            }
            path
        } else if let Some(command) = worktree.which(SERVER_EXECUTABLE) {
            command
        } else {
            return Err(format!(
                "{SERVER_EXECUTABLE} was not found; install the AdocWeave Language Server and add it to PATH, or set lsp.{SERVER_NAME}.binary.path to its absolute path"
            ));
        };
        Ok(server_command(command, worktree.shell_env()))
    }
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
}

zed::register_extension!(AdocWeaveExtension);
