//! One bounded, cancellable helper process per generation.

use super::protocol::{self, Response, ScopeOutput, Scopes};
use super::{HostError, HostResult, Prepared, ValidatedResults, validate_response};
use adocweave_core::CancellationCheck;
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

#[cfg(windows)]
#[path = "windows_job.rs"]
mod windows_job;

#[derive(Clone, Copy, Debug)]
pub struct ProcessLimits {
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub stderr_bytes: usize,
    pub timeout: Duration,
}
impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            input_bytes: protocol::INPUT_BYTES,
            output_bytes: protocol::OUTPUT_BYTES,
            stderr_bytes: 64 * 1024,
            timeout: Duration::from_secs(30),
        }
    }
}

/// A direct executable and literal arguments, never a shell command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelperCommand {
    pub executable: PathBuf,
    pub arguments: Vec<std::ffi::OsString>,
    pub working_directory: PathBuf,
}

/// Only explicit configuration or installed known PATH commands are considered.
pub fn resolve_executable(explicit: Option<&Path>) -> HostResult<HelperCommand> {
    let configured = std::env::var_os("ADOCWEAVE_SLIDES_HELPER");
    let search = std::env::var_os("PATH").unwrap_or_default();
    let paths: Vec<_> = std::env::split_paths(&search)
        .filter(|p| p.is_absolute())
        .collect();
    resolve_from_paths(
        explicit.or_else(|| configured.as_deref().map(Path::new)),
        &paths,
        cfg!(windows),
    )
}
fn resolve_from_paths(
    explicit: Option<&Path>,
    paths: &[PathBuf],
    windows: bool,
) -> HostResult<HelperCommand> {
    if let Some(path) = explicit {
        if path.as_os_str().is_empty() {
            return Err(HostError::new(
                "slides-helper-not-found",
                "helper executable path is empty",
            ));
        }
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|e| HostError::new("slides-helper-io", e.to_string()))?
                .join(path)
        };
        if path.extension().is_some_and(|extension| {
            extension
                .to_str()
                .is_some_and(|value| value.eq_ignore_ascii_case("mjs"))
        }) {
            if !path.is_file() {
                return Err(HostError::new(
                    "slides-helper-not-found",
                    "helper module path does not exist",
                ));
            }
            return node_command(path, paths, windows);
        }
        if windows
            && path.extension().is_some_and(|extension| {
                extension.to_str().is_some_and(|value| {
                    value.eq_ignore_ascii_case("cmd") || value.eq_ignore_ascii_case("bat")
                })
            })
        {
            return Err(HostError::new(
                "slides-helper-not-found",
                "specify the helper bin.mjs or native executable instead of a command shim",
            ));
        }
        if !executable(&path) {
            return Err(HostError::new(
                "slides-helper-not-found",
                format!("helper executable is not available: {}", path.display()),
            ));
        }
        return Ok(HelperCommand {
            working_directory: path
                .parent()
                .expect("absolute executable has a parent")
                .to_owned(),
            executable: path,
            arguments: Vec::new(),
        });
    }
    for directory in paths.iter().filter(|p| p.is_absolute()) {
        let native = directory.join(if windows {
            "adocweave-slides-helper.exe"
        } else {
            "adocweave-slides-helper"
        });
        if executable(&native) {
            // npm's Unix entry is a symlink to bin.mjs. Resolve it before
            // choosing Node so its env shebang cannot search PATH again.
            let resolved = native
                .canonicalize()
                .map_err(|e| HostError::new("slides-helper-io", e.to_string()))?;
            if !windows
                && resolved
                    .extension()
                    .is_some_and(|extension| extension == "mjs")
            {
                return node_command(resolved, paths, false);
            }
            return Ok(HelperCommand {
                working_directory: native
                    .parent()
                    .expect("absolute executable has a parent")
                    .to_owned(),
                executable: native,
                arguments: Vec::new(),
            });
        }
        if windows && directory.join("adocweave-slides-helper.cmd").is_file() {
            let module = directory.join("node_modules/@adocweave/slides-helper/bin.mjs");
            if !module.is_file() {
                return Err(HostError::new(
                    "slides-helper-not-found",
                    "npm helper module is missing from its installation prefix; specify bin.mjs explicitly",
                ));
            }
            return node_command(module, paths, true);
        }
    }
    Err(HostError::new(
        "slides-helper-not-found",
        "install @adocweave/slides-helper or set --slides-helper / ADOCWEAVE_SLIDES_HELPER",
    ))
}
fn node_command(module: PathBuf, paths: &[PathBuf], windows: bool) -> HostResult<HelperCommand> {
    let node = paths
        .iter()
        .filter(|p| p.is_absolute())
        .map(|p| p.join(if windows { "node.exe" } else { "node" }))
        .find(|p| executable(p))
        .ok_or_else(|| {
            HostError::new(
                "slides-helper-node-not-found",
                "Node.js executable is not available on the absolute PATH; install Node.js >=24.19.0 and @adocweave/slides-helper. See https://github.com/KeishiS/adocweave/blob/main/docs/user-guide/release-installation.adoc",
            )
        })?;
    Ok(HelperCommand {
        working_directory: module
            .parent()
            .expect("absolute module has a parent")
            .to_owned(),
        executable: node,
        arguments: vec![module.into_os_string()],
    })
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub async fn execute(
    prepared: &Prepared,
    explicit: Option<&Path>,
    cancellation: &dyn CancellationCheck,
    limits: ProcessLimits,
    reserved_ids: &BTreeSet<String>,
) -> HostResult<ValidatedResults> {
    prepared.request.validate()?;
    if prepared.request.is_empty() {
        return validate_response(
            prepared,
            Response {
                schema_version: 1,
                scopes: Scopes {
                    body: ScopeOutput::default(),
                    notes: ScopeOutput::default(),
                },
                diagnostics: Vec::new(),
                notices: protocol::Notices::default(),
            },
            0,
            reserved_ids,
        );
    }
    if cancellation.is_cancelled() {
        return Err(HostError::new(
            "slides-helper-cancelled",
            "slide generation was cancelled",
        ));
    }
    let mut input = BoundedBuffer {
        bytes: Vec::new(),
        limit: limits.input_bytes.min(protocol::INPUT_BYTES),
    };
    serde_json::to_writer(&mut input, &prepared.request)
        .map_err(|e| HostError::new("slides-helper-input-limit", e.to_string()))?;
    let executable = resolve_executable(explicit)?;
    let path = std::env::join_paths(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .filter(|path| path.is_absolute()),
    )
    .map_err(|e| HostError::new("slides-helper-spawn", e.to_string()))?;
    let mut command = tokio::process::Command::new(executable.executable);
    command
        .args(executable.arguments)
        .current_dir(executable.working_directory)
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Node loader/preload settings are not part of the finite helper request.
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("NODE_")
        {
            command.env_remove(name);
        }
    }
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    #[cfg(windows)]
    let job = windows_job::Job::new().map_err(|e| {
        HostError::new(
            "slides-helper-spawn",
            format!("cannot create helper process job: {e}"),
        )
    })?;
    let mut child = command
        .spawn()
        .map_err(|e| HostError::new("slides-helper-spawn", e.to_string()))?;
    #[cfg(unix)]
    let group = child.id().expect("new child has a process ID") as i32;
    #[cfg(windows)]
    if let Err(error) = job.assign_and_resume(&child) {
        // Assignment/resume failure must never leave a suspended child behind.
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(HostError::new(
            "slides-helper-spawn",
            format!("cannot start helper in its process job: {error}"),
        ));
    }
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let timeout = limits.timeout.min(Duration::from_secs(30));
    let result = {
        let operation = async {
            let (input_result, stdout, stderr, status) = tokio::try_join!(
                async move {
                    let result = async {
                        stdin.write_all(&input.bytes).await?;
                        stdin.shutdown().await
                    }
                    .await;
                    // ChildStdin::shutdown does not close the pipe handle.
                    // The helper reads one JSON document until stdin EOF.
                    drop(stdin);
                    // Early exits can close stdin before reading the request.
                    // Keep reading stderr and waiting for the exit status.
                    Ok::<_, HostError>(result)
                },
                read_bounded(
                    stdout,
                    limits.output_bytes.min(protocol::OUTPUT_BYTES),
                    "stdout"
                ),
                read_bounded(stderr, limits.stderr_bytes.min(64 * 1024), "stderr"),
                async {
                    child
                        .wait()
                        .await
                        .map_err(|e| HostError::new("slides-helper-io", e.to_string()))
                }
            )?;
            Ok((input_result, stdout, stderr, status))
        };
        tokio::pin!(operation);
        tokio::select! {
            result=&mut operation=>result,
            _=tokio::time::sleep(timeout)=>Err(HostError::new("slides-helper-timeout",format!("slide helper exceeded its {timeout:?} time limit; reduce the document's math or citation workload and retry"))),
            _=wait_cancelled(cancellation)=>Err(HostError::new("slides-helper-cancelled","slide generation was cancelled")),
        }
    };
    let (input_result, stdout, stderr, status) = match result {
        Ok(result) => result,
        Err(error) => {
            #[cfg(unix)]
            // The helper started in its own group; inherited descendants share
            // it unless a trusted helper deliberately detaches itself.
            let cleanup = if unsafe { libc::kill(-group, libc::SIGKILL) } == -1 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    Ok(())
                } else {
                    Err(error)
                }
            } else {
                Ok(())
            };
            #[cfg(windows)]
            let cleanup = job.terminate();
            // Reap even after an I/O/limit/cancellation failure, including a child that just exited.
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => HostError::new(
                    error.code,
                    format!(
                        "{}; helper process cleanup failed: {cleanup}",
                        error.message
                    ),
                ),
            });
        }
    };
    let exit = status.code().ok_or_else(|| {
        HostError::new(
            "slides-helper-exit",
            with_stderr("slide helper terminated without an exit code", &stderr),
        )
    })?;
    if !matches!(exit, 0 | 1) {
        return Err(HostError::new(
            "slides-helper-exit",
            with_stderr(&format!("slide helper exited with status {exit}"), &stderr),
        ));
    }
    let mut parser = serde_json::Deserializer::from_slice(&stdout);
    use serde::Deserialize;
    let response = Response::deserialize(&mut parser).map_err(|e| {
        HostError::new(
            if exit == 1 { "slides-helper-exit" } else { "slides-helper-protocol" },
            with_stderr(&format!(
                "slide helper exited with status {exit} without a protocol response; helper stdout must contain one protocol JSON object: {e}"
            ), &stderr),
        )
    })?;
    parser.end().map_err(|e| {
        HostError::protocol(with_stderr(&format!("slide helper exited with status {exit}; helper stdout has logging or additional JSON: {e}"), &stderr))
    })?;
    input_result.map_err(|error| HostError::new(
        "slides-helper-io",
        with_stderr(&format!("could not send the complete request to the slide helper (exit status {exit}): {error}"), &stderr),
    ))?;
    validate_response(prepared, response, exit, reserved_ids)
}

fn with_stderr(message: &str, stderr: &[u8]) -> String {
    if stderr.is_empty() {
        return message.to_owned();
    }
    let tail = String::from_utf8_lossy(&stderr[stderr.len().saturating_sub(8192)..]);
    let lines = tail.lines().rev().take(12).collect::<Vec<_>>();
    let mut result = format!("{message}\nhelper stderr (last lines):");
    for line in lines.into_iter().rev() {
        result.push_str("\n  ");
        for character in line.chars() {
            if character.is_control() && character != '\t' {
                result.extend(character.escape_default());
            } else {
                result.push(character);
            }
        }
    }
    result
}

/// A worker may call this without maintaining a runtime or process registry.
/// The scoped thread also permits a synchronous host already entered into Tokio.
pub fn execute_sync(
    prepared: &Prepared,
    explicit: Option<&Path>,
    cancellation: &dyn CancellationCheck,
    limits: ProcessLimits,
    reserved_ids: &BTreeSet<String>,
) -> HostResult<ValidatedResults> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| HostError::new("slides-helper-io", e.to_string()))?
                    .block_on(execute(
                        prepared,
                        explicit,
                        cancellation,
                        limits,
                        reserved_ids,
                    ))
            })
            .join()
            .map_err(|_| HostError::new("slides-helper-io", "slide helper worker panicked"))?
    })
}
async fn wait_cancelled(cancellation: &dyn CancellationCheck) {
    loop {
        if cancellation.is_cancelled() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn read_bounded(
    mut reader: impl AsyncRead + Unpin,
    limit: usize,
    stream: &str,
) -> HostResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let count = reader.read(&mut chunk).await.map_err(|e| {
            HostError::new(
                "slides-helper-io",
                format!("cannot read helper {stream}: {e}"),
            )
        })?;
        if count == 0 {
            return Ok(bytes);
        }
        if count > limit.saturating_sub(bytes.len()) {
            return Err(HostError::new(
                if stream == "stderr" {
                    "slides-helper-stderr-limit"
                } else {
                    "slides-helper-output-limit"
                },
                format!(
                    "helper {stream} exceeded its {limit}-byte limit; check the helper installation"
                ),
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}
struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl io::Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "helper input byte limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    #[test]
    fn windows_npm_prefix_and_node_are_resolved_as_literal_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("npm prefix with spaces");
        let node_dir = temp.path().join("Node Program Files");
        file(&prefix.join("adocweave-slides-helper.cmd"));
        let module = prefix.join("node_modules/@adocweave/slides-helper/bin.mjs");
        file(&module);
        file(&node_dir.join("node.exe"));
        let command = resolve_from_paths(None, &[prefix.clone(), node_dir.clone()], true).unwrap();
        assert_eq!(command.executable, node_dir.join("node.exe"));
        assert_eq!(command.arguments, [module.clone().into_os_string()]);
        assert_eq!(
            resolve_from_paths(Some(&module), &[node_dir], true).unwrap(),
            command
        );
        assert_eq!(
            resolve_from_paths(None, &[prefix], true).unwrap_err().code,
            "slides-helper-node-not-found"
        );
    }
    #[test]
    fn windows_shim_does_not_trigger_project_or_parent_directory_search() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("npm");
        file(&prefix.join("adocweave-slides-helper.cmd"));
        file(
            &temp
                .path()
                .join("node_modules/@adocweave/slides-helper/bin.mjs"),
        );
        file(&temp.path().join("node.exe"));
        assert_eq!(
            resolve_from_paths(None, &[prefix.clone(), temp.path().to_owned()], true)
                .unwrap_err()
                .code,
            "slides-helper-not-found"
        );
        assert!(
            resolve_from_paths(Some(&prefix.join("adocweave-slides-helper.cmd")), &[], true)
                .is_err()
        );
    }
}
