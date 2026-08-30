//! Typed Language Server service and transport tests.

use async_lsp::lsp_types as lsp;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::{PositionEncoding, StdioError, StdioErrorKind, run};
use crate::service::Session;
use crate::state::ProjectAnalysisSnapshot;

mod support;

use support::*;

mod capabilities;
mod diagnostics;
mod editing;
mod feature_integration;
mod navigation;
mod project;
mod protocol;
mod semantic_tokens;
mod session;
