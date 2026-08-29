//! Allocation-owning WebAssembly boundary over the deterministic core.

use adocweave::preprocess::{
    PreprocessErrorKind, PreprocessInputs, PreprocessedAnalysisError, ProjectionFailure,
    ProjectionLimits,
};
use adocweave::{CancellationCheck, NeverCancel, ParseError};

mod preprocess_wire;
mod protocol;
mod render_input_conversion;
mod render_input_normalization;
mod render_input_wire;
mod request_conversion;
mod request_enums;
mod request_wire;
mod response_conversion;
mod response_projection;
mod response_wire;
mod shared_wire;

pub use preprocess_wire::{AdocWeaveError, SafeMode};
pub use protocol::PROTOCOL_SCHEMA_VERSION;
pub use render_input_wire::{
    CitationOutcome, CitationSegment, GeneratedBibliography, GeneratedBibliographyEntry,
    ReferenceFailureKind, ReferenceNotice, ReferenceOutcome, ResolvedCitation, ResolvedReference,
    ResolvedResource, ResourceFailureKind, ResourceOutcome,
};
pub use request_enums::{
    DocumentMode, SyntaxMode, UnknownRole, UnknownSourceLanguage, UnresolvedReferencePresentation,
};
pub use request_wire::{
    ActiveUrlOptions, AnalyzeRequest, AuthoredUrlOptions, DiagnosticOptions, ExternalLinkOptions,
    HtmlOptions, IncludeHandling, ProductRequest, ResourceCapabilities, ResourceInput, RoleOptions,
    RuleOptions, SourceInput, SourceLanguageOptions, Stylesheet,
};
use response_conversion::wasm_document_projection;
use response_projection::{ResponseProducts, enforce_output_limit, project_response};
pub use response_wire::*;
pub use shared_wire::{MathLanguage, Severity};

pub fn analyze_request(
    request: AnalyzeRequest,
    cancellation: &dyn CancellationCheck,
) -> Result<AnalyzeResult, AdocWeaveError> {
    let request = request_conversion::convert(request)?;
    execute_request(request, cancellation)
}

fn execute_request(
    request: request_conversion::ExecutionRequest,
    cancellation: &dyn CancellationCheck,
) -> Result<AnalyzeResult, AdocWeaveError> {
    let analysis = request
        .processing_options
        .preprocess_and_analyze(
            &request.source,
            &request.snapshot,
            PreprocessInputs {
                cancellation: Some(cancellation),
            },
        )
        .map_err(preprocessed_analysis_error)?;
    let requested = request.requested_products;
    let attribute_projection = if requested.attribute_queries.is_some() {
        Some(
            analysis
                .project_origins_cancellable(ProjectionLimits::default(), cancellation)
                .map_err(|error| match error {
                    ProjectionFailure::LimitExceeded(error) => AdocWeaveError {
                        code: "input-limit-exceeded".to_owned(),
                        message: error.to_string(),
                    },
                    ProjectionFailure::Cancelled => cancelled_error(),
                })?,
        )
    } else {
        None
    };
    if cancellation.is_cancelled() {
        return Err(cancelled_error());
    }

    let render_inputs =
        render_input_conversion::convert(request.render_inputs, &analysis.analysis)?;
    let html = requested.html.as_ref().map(|_| {
        adocweave::output::html::render_with_inputs(
            analysis.analysis.document(),
            &request.render_policy,
            &render_inputs,
        )
    });
    let products = ResponseProducts {
        syntax: requested
            .syntax
            .map(|()| adocweave::output::canonical::canonical_syntax(&analysis.analysis)),
        canonical_ast: requested
            .canonical_ast
            .map(|()| adocweave::output::canonical::canonical_ast(&analysis.analysis)),
        html,
        attribute_occurrences: requested
            .attribute_occurrences
            .map(|()| analysis.analysis.document_attribute_occurrences().to_vec()),
        attribute_queries: requested
            .attribute_queries
            .map(|()| analysis.analysis.attribute_query_product()),
        resource_queries: requested
            .resource_queries
            .map(|()| analysis.analysis.resource_queries()),
        diagnostics: requested
            .diagnostics
            .map(|_| analysis.analysis.diagnostics().to_vec()),
        symbols: requested
            .symbols
            .map(|()| adocweave::semantic::document_symbols(analysis.analysis.document())),
        document: requested
            .document
            .map(|()| wasm_document_projection(&analysis.analysis, &render_inputs)),
    };
    if cancellation.is_cancelled() {
        return Err(cancelled_error());
    }
    let response = project_response(
        products,
        request.source_id.as_ref(),
        attribute_projection.as_ref(),
    )?;
    if cancellation.is_cancelled() {
        return Err(cancelled_error());
    }
    enforce_output_limit(&response, request.max_output_bytes)?;
    Ok(response)
}

pub fn analyze_json(request: &str) -> Result<String, String> {
    let request = serde_json::from_str(request).map_err(|error| {
        serialize_error(&request_conversion::invalid_request(error.to_string()))
    })?;
    analyze_request(request, &NeverCancel)
        .and_then(|response| serde_json::to_string(&response).map_err(serialization_error))
        .map_err(|error| serialize_error(&error))
}

fn parse_error(error: ParseError) -> AdocWeaveError {
    let code = match error {
        ParseError::LimitExceeded { .. } => "input-limit-exceeded",
        ParseError::Cancelled => "cancelled",
        _ => "analysis-failed",
    };
    AdocWeaveError {
        code: code.to_owned(),
        message: error.to_string(),
    }
}

fn preprocessed_analysis_error(error: PreprocessedAnalysisError) -> AdocWeaveError {
    match error {
        PreprocessedAnalysisError::Options(error) => {
            request_conversion::invalid_request(error.to_string())
        }
        PreprocessedAnalysisError::Preprocess(error) => {
            let code = match error.kind {
                PreprocessErrorKind::DepthLimit
                | PreprocessErrorKind::IncludeLimit
                | PreprocessErrorKind::ByteLimit
                | PreprocessErrorKind::NodeLimit
                | PreprocessErrorKind::SourceMapLimit => "input-limit-exceeded",
                _ => "analysis-failed",
            };
            AdocWeaveError {
                code: code.to_owned(),
                message: error.to_string(),
            }
        }
        PreprocessedAnalysisError::Parse(error) => parse_error(error),
        PreprocessedAnalysisError::Cancelled => cancelled_error(),
    }
}

fn cancelled_error() -> AdocWeaveError {
    AdocWeaveError {
        code: "cancelled".to_owned(),
        message: "analysis was cancelled".to_owned(),
    }
}

pub(crate) fn serialization_error(error: impl ToString) -> AdocWeaveError {
    AdocWeaveError {
        code: "analysis-failed".to_owned(),
        message: error.to_string(),
    }
}

fn serialize_error(error: &AdocWeaveError) -> String {
    serde_json::to_string(error).unwrap_or_else(|_| {
        "{\"code\":\"analysis-failed\",\"message\":\"failed to serialize error\"}".to_owned()
    })
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    use js_sys::{Array, Object, Reflect, Set};
    use serde::Serialize as _;
    use wasm_bindgen::prelude::*;

    use super::*;

    #[wasm_bindgen(js_name = protocolSchemaVersion)]
    pub fn protocol_schema_version_js() -> u16 {
        PROTOCOL_SCHEMA_VERSION
    }

    #[wasm_bindgen(js_name = analyze)]
    pub fn analyze_js(request: JsValue) -> Result<JsValue, JsValue> {
        let request = serde_input(request)
            .and_then(|request| {
                serde_wasm_bindgen::from_value(request)
                    .map_err(|error| request_conversion::invalid_request(error.to_string()))
            })
            .map_err(error_to_js)?;
        let response = analyze_request(request, &NeverCancel).map_err(error_to_js)?;
        response
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .map_err(|error| error_to_js(serialization_error(error)))
    }

    fn error_to_js(error: AdocWeaveError) -> JsValue {
        serde_wasm_bindgen::to_value(&error)
            .unwrap_or_else(|_| JsValue::from_str("failed to serialize error"))
    }

    fn serde_input(value: JsValue) -> Result<JsValue, AdocWeaveError> {
        normalize_value(value, &Set::new(&JsValue::UNDEFINED), 0)
    }

    fn normalize_value(
        value: JsValue,
        ancestors: &Set,
        depth: u16,
    ) -> Result<JsValue, AdocWeaveError> {
        if depth >= 128 {
            return Err(request_conversion::invalid_request(
                "request objects must not exceed 128 levels",
            ));
        }
        if Array::is_array(&value) {
            if ancestors.has(&value) {
                return Err(request_conversion::invalid_request(
                    "request must not contain a cycle",
                ));
            }
            ancestors.add(&value);
            let input = Array::from(&value);
            let output = Array::new_with_length(input.length());
            for index in 0..input.length() {
                output.set(
                    index,
                    normalize_value(input.get(index), ancestors, depth + 1)?,
                );
            }
            ancestors.delete(&value);
            return Ok(output.into());
        }
        if value.is_object() && !value.is_null() {
            if ancestors.has(&value) {
                return Err(request_conversion::invalid_request(
                    "request must not contain a cycle",
                ));
            }
            ancestors.add(&value);
            let output = Object::new();
            for field in Object::keys(value.unchecked_ref::<Object>()).iter() {
                let field_value = Reflect::get(&value, &field).map_err(|_| {
                    request_conversion::invalid_request("request fields must be readable")
                })?;
                if !field_value.is_undefined() {
                    Reflect::set(
                        &output,
                        &field,
                        &normalize_value(field_value, ancestors, depth + 1)?,
                    )
                    .map_err(|_| {
                        request_conversion::invalid_request("request fields must be writable")
                    })?;
                }
            }
            ancestors.delete(&value);
            return Ok(output.into());
        }
        Ok(value)
    }
}
