use adocweave::{CancellationToken, NeverCancel};
use adocweave_wasm::{AdocWeaveError, AnalyzeRequest, analyze_json, analyze_request};
use serde_json::{Value, json};

fn analyze(value: Value) -> Result<Value, AdocWeaveError> {
    let request: AnalyzeRequest = serde_json::from_value(value).expect("valid request");
    analyze_request(request, &NeverCancel).and_then(|response| {
        serde_json::to_value(response).map_err(|error| AdocWeaveError {
            code: "analysis-failed".to_owned(),
            message: error.to_string(),
        })
    })
}

#[test]
fn request_requires_source_text_products_and_a_selected_product() {
    for value in [
        json!({ "products": { "html": true } }),
        json!({ "source": { "text": "Text" } }),
    ] {
        assert!(serde_json::from_value::<AnalyzeRequest>(value).is_err());
    }

    let request: AnalyzeRequest = serde_json::from_value(json!({
        "source": { "text": "Text" },
        "products": {}
    }))
    .expect("syntactically valid request");
    assert_eq!(
        analyze_request(request, &NeverCancel)
            .expect_err("empty products")
            .code,
        "invalid-request"
    );
}

#[test]
fn unknown_fields_null_false_and_invalid_values_are_rejected() {
    for value in [
        json!({ "source": { "text": "Text", "unknown": 1 }, "products": { "html": true } }),
        json!({ "source": { "text": "Text" }, "products": { "html": true }, "unknown": 1 }),
        json!({ "source": { "text": "Text", "id": null }, "products": { "html": true } }),
        json!({ "source": { "text": "Text" }, "products": { "html": null } }),
        json!({ "source": { "text": "Text" }, "products": { "html": false } }),
        json!({ "source": { "text": "Text" }, "products": { "html": true }, "resources": null }),
        json!({ "source": { "text": "Text", "syntaxMode": "unknown" }, "products": { "html": true } }),
        json!({ "source": { "text": "Text" }, "products": { "html": { "activeUrls": { "unknown": true } } } }),
        json!({ "source": { "text": "Text" }, "products": { "html": true }, "resources": { "unknown": true } }),
    ] {
        assert!(
            serde_json::from_value::<AnalyzeRequest>(value).is_err(),
            "invalid value was accepted"
        );
    }

    for value in [
        json!({ "source": { "text": "Text", "id": "" }, "products": { "html": true } }),
        json!({ "source": { "text": "Text" }, "products": { "diagnostics": { "maxDiagnostics": 1001 } } }),
        json!({ "source": { "text": "Text" }, "products": { "html": true }, "resources": { "allowedSchemes": ["https", "HTTPS"] } }),
    ] {
        let request = serde_json::from_value(value).expect("structurally valid request");
        assert_eq!(
            analyze_request(request, &NeverCancel)
                .expect_err("invalid request")
                .code,
            "invalid-request"
        );
    }
}

#[test]
fn response_contains_only_requested_products() {
    let response = analyze(json!({
        "source": { "text": "= Title\n\nText" },
        "products": { "html": true, "symbols": true, "document": true }
    }))
    .expect("response");
    let object = response.as_object().expect("object response");
    assert_eq!(
        object
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        ["document", "html", "symbols"].into_iter().collect()
    );
    assert!(
        response["html"]
            .as_str()
            .is_some_and(|html| html.contains("<h1"))
    );
    assert_eq!(response["symbols"][0]["name"], "Title");
    assert!(response["document"].is_object());
}

#[test]
fn html_and_diagnostics_options_share_the_same_request() {
    let response = analyze(json!({
        "source": { "text": "link:javascript:alert(1)[unsafe]" },
        "products": {
            "html": { "documentMode": "fragment" },
            "diagnostics": { "maxDiagnostics": 10 }
        }
    }))
    .expect("response");
    assert_eq!(response.as_object().expect("object").len(), 2);
    assert!(response["diagnostics"].is_array());
}

#[test]
fn resources_expand_includes_and_supply_render_inputs() {
    let response = analyze(json!({
        "source": { "text": "include::chapter.adoc[]", "id": "main.adoc" },
        "products": { "html": true, "syntax": true },
        "resources": {
            "documents": { "chapter.adoc": "== Included\n" },
            "includes": "expand"
        }
    }))
    .expect("response");
    assert!(
        response["html"]
            .as_str()
            .is_some_and(|html| html.contains("Included"))
    );
}

#[test]
fn native_json_entry_uses_the_common_error_shape() {
    let error = analyze_json(r#"{"source":{"text":"Text"},"products":{"html":false}}"#)
        .expect_err("invalid request");
    let error: AdocWeaveError = serde_json::from_str(&error).expect("error JSON");
    assert_eq!(error.code, "invalid-request");
    assert!(!error.message.is_empty());
}

#[test]
fn cancellation_uses_the_common_error_code() {
    let request = serde_json::from_value(json!({
        "source": { "text": "Text" },
        "products": { "html": true }
    }))
    .expect("request");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        analyze_request(request, &cancellation)
            .expect_err("cancelled")
            .code,
        "cancelled"
    );
}

#[test]
fn byte_ranges_use_utf8_encoded_offsets() {
    let response = analyze(json!({
        "source": { "text": "= 文書\n" },
        "products": { "symbols": true }
    }))
    .expect("response");
    assert_eq!(response["symbols"][0]["range"]["end"], 9);
}
