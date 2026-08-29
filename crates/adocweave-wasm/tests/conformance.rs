use adocweave::{AnalysisOptions, Engine, NeverCancel};
use adocweave_wasm::{AnalyzeRequest, analyze_request};
use serde_json::json;

#[path = "../src/canonical.rs"]
mod canonical;

#[test]
fn native_and_wasm_boundary_produce_the_same_canonical_products() {
    let source = "= Title\n\n== Section\n\nText *strong*.\n";
    let native = Engine::new(AnalysisOptions::default())
        .analyze(source)
        .expect("native analysis");
    let request: AnalyzeRequest = serde_json::from_value(json!({
        "source": { "text": source },
        "products": { "syntax": true, "canonicalAst": true }
    }))
    .expect("request");
    let boundary = analyze_request(request, &NeverCancel).expect("boundary analysis");

    assert_eq!(
        boundary.syntax.as_deref(),
        Some(canonical::canonical_syntax(&native).as_str())
    );
    assert_eq!(
        boundary.canonical_ast.as_deref(),
        Some(canonical::canonical_ast(&native).as_str())
    );
}

#[test]
fn native_and_wasm_boundary_produce_the_same_html() {
    let source = "= Title\n\nText *strong*.\n";
    let native = Engine::new(AnalysisOptions::default())
        .analyze(source)
        .expect("native analysis");
    let expected = adocweave::output::html::render(
        native.document(),
        &adocweave::output::html::RenderPolicy::default(),
    )
    .html;
    let request: AnalyzeRequest = serde_json::from_value(json!({
        "source": { "text": source },
        "products": { "html": true }
    }))
    .expect("request");
    let boundary = analyze_request(request, &NeverCancel).expect("boundary analysis");

    assert_eq!(boundary.html.as_deref(), Some(expected.as_str()));
}
