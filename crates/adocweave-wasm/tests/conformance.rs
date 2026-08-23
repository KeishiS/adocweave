use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use adocweave::NeverCancel;
use adocweave_wasm::{WasmRequest, process_request};
use serde_json::{Value, json};

#[test]
fn declared_inline_error_and_public_contracts_match_adapter_outputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures = root.join("fixtures/conformance");
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(root.join("crates/adocweave/conformance/cases.json"))
            .expect("conformance manifest"),
    )
    .expect("valid conformance manifest");
    assert_eq!(manifest["schemaVersion"], 2, "manifest schema version");
    assert_eq!(
        manifest["outputContractVersion"], 1,
        "public output contract version"
    );
    assert_eq!(manifest["license"], "MIT OR Apache-2.0");
    assert!(
        manifest["globalImplementationDetails"]
            .as_array()
            .is_some_and(|details| !details.is_empty()),
        "global implementation details"
    );
    let mut public_case_count = 0;
    let mut public_features = BTreeSet::new();
    let mut case_names = BTreeSet::new();
    let mut public_source_ids = BTreeSet::new();

    for entry in manifest["cases"].as_array().expect("cases") {
        let name = entry["name"].as_str().expect("case name");
        assert!(!name.is_empty(), "case name must not be empty");
        assert!(case_names.insert(name), "duplicate case name: {name}");
        assert!(entry["compatibility"].is_string(), "{name}: compatibility");
        assert!(entry["rationale"].is_string(), "{name}: rationale");
        assert!(
            entry["contractImpact"].is_string(),
            "{name}: contract impact"
        );
        for (file, inline) in [
            ("expectedHtmlFile", "expectedHtml"),
            ("expectedAstFile", "expectedAst"),
            ("expectedDiagnosticsFile", "expectedDiagnostics"),
            ("expectedRenderDiagnosticsFile", "expectedRenderDiagnostics"),
            ("expectedProjectionFile", "expectedProjection"),
            ("expectedSymbolsFile", "expectedSymbols"),
        ] {
            assert_exclusive_expectation(entry, file, inline, name);
        }

        let has_inline_expectation = [
            "expectedHtml",
            "expectedAst",
            "expectedDiagnostics",
            "expectedRenderDiagnostics",
            "expectedProjection",
            "expectedSymbols",
        ]
        .iter()
        .any(|field| entry.get(field).is_some());
        if !entry["expectedErrorCode"].is_string()
            && entry.get("publicContract").is_none()
            && !has_inline_expectation
        {
            continue;
        }

        let result = process_request(request_for(entry, &fixtures), &NeverCancel);

        if let Some(code) = entry["expectedErrorCode"].as_str() {
            assert_eq!(result.expect_err(name).code, code, "{name}");
            continue;
        }
        let response = result.expect(name);
        for (field, actual) in [
            ("expectedHtml", Value::String(response.html.clone())),
            ("expectedAst", Value::String(response.ast.clone())),
            (
                "expectedDiagnostics",
                serde_json::to_value(&response.diagnostics).expect("diagnostics JSON"),
            ),
            (
                "expectedRenderDiagnostics",
                serde_json::to_value(&response.render_diagnostics)
                    .expect("render diagnostics JSON"),
            ),
            (
                "expectedProjection",
                serde_json::to_value(&response.projection).expect("projection JSON"),
            ),
            (
                "expectedSymbols",
                serde_json::to_value(&response.symbols).expect("symbols JSON"),
            ),
        ] {
            if let Some(expected) = entry.get(field) {
                assert_eq!(&actual, expected, "{name}: {field}");
            }
        }
        if let Some(public_contract) = entry.get("publicContract") {
            public_case_count += 1;
            let source_id = entry["sourceId"].as_str().expect("public sourceId");
            assert!(!source_id.is_empty(), "{name}: public sourceId");
            assert!(
                public_source_ids.insert(source_id),
                "duplicate public sourceId: {source_id}"
            );
            assert_eq!(
                entry["analysisOptions"],
                json!({}),
                "{name}: analysis options"
            );
            assert_eq!(
                entry["renderPolicy"],
                json!({ "documentMode": "fragment" }),
                "{name}: render policy"
            );
            assert_public_contract(
                name,
                public_contract,
                &response.html,
                &serde_json::to_value(&response.projection).expect("projection JSON"),
                &serde_json::to_value(&response.diagnostics).expect("diagnostics JSON"),
                &serde_json::to_value(&response.render_diagnostics)
                    .expect("render diagnostics JSON"),
                &mut public_features,
            );
        }
    }
    assert_eq!(public_case_count, 6, "public conformance case count");
    for required in [
        "document-title",
        "toc",
        "section-numbers",
        "source-block",
        "block-title",
        "source-language-option",
        "source-line-numbers",
        "source-start-line",
        "unsupported-source-option",
        "inline-formula",
        "block-formula",
        "table",
        "quote",
        "unordered-list",
        "unsafe-url",
        "diagnostic",
    ] {
        assert!(
            public_features.contains(required),
            "missing public feature: {required}"
        );
    }
}

fn assert_exclusive_expectation(entry: &Value, file: &str, inline: &str, name: &str) {
    assert!(
        entry.get(file).is_none() || entry.get(inline).is_none(),
        "{name}: {file} and {inline} are mutually exclusive"
    );
}

fn assert_public_contract<'a>(
    name: &str,
    contract: &'a Value,
    html: &str,
    projection: &Value,
    diagnostics: &Value,
    render_diagnostics: &Value,
    features: &mut BTreeSet<&'a str>,
) {
    assert_object_keys(
        contract,
        &["features", "implementationDetails", "stableContract"],
        &format!("{name}: publicContract"),
    );
    let case_features = contract["features"].as_array().expect("public features");
    assert!(!case_features.is_empty(), "{name}: public features");
    for feature in case_features {
        features.insert(feature.as_str().expect("public feature name"));
    }
    assert!(
        contract["implementationDetails"]
            .as_array()
            .is_some_and(|details| !details.is_empty()),
        "{name}: implementation details"
    );
    let stable = &contract["stableContract"];
    assert_object_keys(
        stable,
        &[
            "diagnosticCodes",
            "htmlContains",
            "projectionAssertions",
            "renderDiagnosticCodes",
        ],
        &format!("{name}: stableContract"),
    );
    let projection_assertions = stable["projectionAssertions"]
        .as_array()
        .expect("projection assertions");
    assert!(
        !projection_assertions.is_empty(),
        "{name}: projection assertions"
    );
    for assertion in projection_assertions {
        assert_object_keys(
            assertion,
            &["pointer", "value"],
            &format!("{name}: projection assertion"),
        );
        let pointer = assertion["pointer"].as_str().expect("JSON pointer");
        assert_eq!(
            projection.pointer(pointer),
            Some(&assertion["value"]),
            "{name}: projection pointer {pointer}"
        );
    }
    let html_assertions = stable["htmlContains"].as_array().expect("HTML assertions");
    assert!(!html_assertions.is_empty(), "{name}: HTML assertions");
    for fragment in html_assertions {
        let fragment = fragment.as_str().expect("HTML fragment");
        assert!(
            html.contains(fragment),
            "{name}: missing HTML fragment: {fragment}"
        );
    }
    assert_eq!(
        diagnostic_codes(diagnostics),
        expected_codes(&stable["diagnosticCodes"]),
        "{name}: diagnostic codes"
    );
    assert_eq!(
        diagnostic_codes(render_diagnostics),
        expected_codes(&stable["renderDiagnosticCodes"]),
        "{name}: render diagnostic codes"
    );
}

fn assert_object_keys(value: &Value, expected: &[&str], context: &str) {
    let actual = value
        .as_object()
        .unwrap_or_else(|| panic!("{context} must be an object"))
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "{context}: fields");
}

fn diagnostic_codes(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .expect("diagnostics array")
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str())
        .collect()
}

fn expected_codes(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .expect("expected diagnostic codes")
        .iter()
        .map(|code| code.as_str().expect("diagnostic code"))
        .collect()
}

/// Builds the expected file contents of one case from the current implementation.
///
/// Regeneration and the cleanliness check share this function so a fixture can
/// never disagree with the command that writes it. Cases that expect an error,
/// and products a case does not name, contribute nothing.
///
fn expected_products(entry: &Value, fixtures: &Path) -> Vec<(PathBuf, String)> {
    let fields = [
        "expectedHtmlFile",
        "expectedAstFile",
        "expectedDiagnosticsFile",
        "expectedRenderDiagnosticsFile",
        "expectedProjectionFile",
        "expectedSymbolsFile",
    ];
    if entry["expectedErrorCode"].is_string()
        || !fields.iter().any(|field| entry[*field].is_string())
    {
        return Vec::new();
    }
    let response = process_request(request_for(entry, fixtures), &NeverCancel)
        .unwrap_or_else(|error| panic!("{}: {}", entry["name"], error.code));
    let mut products = Vec::new();
    let mut text = |field: &str, content: String| {
        if let Some(file) = entry[field].as_str() {
            products.push((resolve(fixtures, file), content));
        }
    };
    // The HTML fixture holds the rendered document exactly; every other product
    // is JSON, written sorted and indented so a review reads the difference
    // rather than one long line.
    text("expectedHtmlFile", response.html);
    text("expectedAstFile", format!("{}\n", response.ast));
    for (field, value) in [
        (
            "expectedDiagnosticsFile",
            serde_json::to_value(&response.diagnostics),
        ),
        (
            "expectedRenderDiagnosticsFile",
            serde_json::to_value(&response.render_diagnostics),
        ),
        (
            "expectedProjectionFile",
            serde_json::to_value(&response.projection),
        ),
        (
            "expectedSymbolsFile",
            serde_json::to_value(&response.symbols),
        ),
    ] {
        let value = value.expect("product serializes as JSON");
        let json = canonical_json(&value);
        text(field, format!("{json}\n"));
    }
    products
}

/// Renders one product as the bytes a fixture stores.
fn canonical_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("JSON product formats")
}

fn shared_cases() -> (Vec<Value>, PathBuf) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(root.join("crates/adocweave/conformance/cases.json"))
            .expect("conformance manifest"),
    )
    .expect("valid conformance manifest");
    let cases = manifest["cases"].as_array().expect("cases").clone();
    (cases, root.join("fixtures/conformance"))
}

#[test]
fn file_backed_products_match_declared_contracts() {
    let (cases, fixtures) = shared_cases();
    for entry in &cases {
        for (path, expected) in expected_products(entry, &fixtures) {
            assert_eq!(
                fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}")),
                expected,
                "{}: {} differs from regeneration",
                entry["name"],
                path.display()
            );
        }
    }
}

/// Rewrites every shared fixture from the current implementation.
///
/// Run this after a deliberate contract change, then read each difference. The
/// command is not part of any gate: a golden fixture exists to show what a
/// change did to the published contract, and overwriting it unread discards
/// exactly that.
#[test]
#[ignore = "fixture maintainer command"]
fn regenerate_shared_fixture_products() {
    let (cases, fixtures) = shared_cases();
    for entry in &cases {
        for (path, content) in expected_products(entry, &fixtures) {
            fs::write(&path, content).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        }
    }
}

fn request_for(entry: &Value, fixtures: &Path) -> WasmRequest {
    let source = entry["sourceFile"].as_str().map_or_else(
        || entry["source"].as_str().expect("inline source").to_owned(),
        |file| fs::read_to_string(resolve(fixtures, file)).expect("fixture source"),
    );
    let analysis_options = entry
        .get("analysisOptions")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let render_policy = entry
        .get("renderPolicy")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let output_limits = entry
        .get("outputLimits")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let render_inputs = entry
        .get("renderInputs")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let preprocess = entry.get("preprocess").cloned().unwrap_or(Value::Null);
    serde_json::from_value(json!({
        "sourceId": entry["sourceId"].as_str().map_or_else(
            || format!("conformance:{}", entry["name"].as_str().expect("name")),
            str::to_owned,
        ),
        "source": source,
        "preprocess": preprocess,
        "products": {
            "syntax": true,
            "canonicalAst": true,
            "html": true,
            "attributeOccurrences": true,
            "attributeQueries": true,
            "resourceQueries": true,
            "diagnostics": true,
            "symbols": true,
            "projection": true,
        },
        "renderInputs": render_inputs,
        "analysisOptions": analysis_options,
        "renderPolicy": render_policy,
        "outputLimits": output_limits,
    }))
    .expect("manifest produces a valid WASM request")
}

fn resolve(base: &Path, path: &str) -> PathBuf {
    base.join(path)
}
