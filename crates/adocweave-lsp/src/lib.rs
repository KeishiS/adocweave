//! Typed LSP adapter, isolated from the deterministic parsing core.

use std::error::Error;
use std::fmt;

use adocweave_host::ExitStatus;

mod backend;
mod cancellation;
mod diagnostics;
mod document_symbols;
mod editing;
mod hover;
mod lifecycle;
mod navigation;
mod position;
mod presentation;
mod semantic_tokens;
mod service;
mod state;
mod workspace;
#[allow(dead_code)]
mod workspace_scan;

pub use position::PositionEncoding;
pub use service::{HostReferenceIndex, HostReferenceRequest, NoHostReferenceIndex};

pub const SERVER_NAME: &str = "adocweave";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A failure while serving the Language Server Protocol over standard I/O.
#[derive(Debug)]
pub struct StdioError {
    source: async_lsp::Error,
}

impl StdioError {
    fn new(source: async_lsp::Error) -> Self {
        Self { source }
    }

    /// Returns the process status required by the protocol or transport failure.
    pub fn exit_status(&self) -> ExitStatus {
        if matches!(self.source, async_lsp::Error::Protocol(_)) {
            ExitStatus::Diagnostics
        } else {
            ExitStatus::InputOutput
        }
    }
}

impl fmt::Display for StdioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(formatter)
    }
}

impl Error for StdioError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

pub async fn run<R, W>(input: R, output: W) -> async_lsp::Result<()>
where
    R: futures::AsyncRead + Unpin,
    W: futures::AsyncWrite + Unpin,
{
    let (main_loop, _) = async_lsp::MainLoop::new_server(backend::Backend::router);
    main_loop.run_buffered(input, output).await
}

pub async fn run_with_host_index<R, W>(
    input: R,
    output: W,
    host_index: std::sync::Arc<dyn HostReferenceIndex>,
) -> async_lsp::Result<()>
where
    R: futures::AsyncRead + Unpin,
    W: futures::AsyncWrite + Unpin,
{
    let (main_loop, _) = async_lsp::MainLoop::new_server(move |client| {
        backend::Backend::router_with_index(client, host_index)
    });
    main_loop.run_buffered(input, output).await
}

pub async fn run_stdio() -> Result<(), StdioError> {
    #[cfg(unix)]
    let (stdin, stdout) = (
        async_lsp::stdio::PipeStdin::lock_tokio()
            .map_err(async_lsp::Error::Io)
            .map_err(StdioError::new)?,
        async_lsp::stdio::PipeStdout::lock_tokio()
            .map_err(async_lsp::Error::Io)
            .map_err(StdioError::new)?,
    );
    #[cfg(not(unix))]
    let (stdin, stdout) = {
        use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
        (
            tokio::io::stdin().compat(),
            tokio::io::stdout().compat_write(),
        )
    };

    run(stdin, stdout).await.map_err(StdioError::new)
}

#[cfg(test)]
mod tests;
