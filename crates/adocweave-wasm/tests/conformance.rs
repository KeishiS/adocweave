use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use adocweave::NeverCancel;
use adocweave_wasm::{WasmRequest, process_request};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Toolchains {
    schema_version: u16,
    rust_version: String,
    node_version: String,
}

#[derive(Deserialize)]
struct BrowserManifest {
    version: String,
}

#[test]
fn native_adapter_accepts_every_shared_conformance_case() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures = root.join("fixtures/conformance");
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(root.join("crates/adocweave/conformance/cases.json"))
            .expect("conformance manifest"),
    )
    .expect("valid conformance manifest");
    assert_eq!(manifest["schemaVersion"], 1, "manifest schema version");
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

    for entry in manifest["cases"].as_array().expect("cases") {
        let name = entry["name"].as_str().expect("case name");
        assert!(entry["compatibility"].is_string(), "{name}: compatibility");
        assert!(entry["rationale"].is_string(), "{name}: rationale");
        assert!(
            entry["contractImpact"].is_string(),
            "{name}: contract impact"
        );
        let request = request_for(entry, &fixtures);
        let result = process_request(request, &NeverCancel);

        if let Some(code) = entry["expectedErrorCode"].as_str() {
            assert_eq!(result.expect_err(name).code, code, "{name}");
            continue;
        }
        let response = result.expect(name);
        assert!(!response.syntax.is_empty(), "{name}: syntax tree");
        assert!(!response.ast.is_empty(), "{name}: AST");
        if let Some(public_contract) = entry.get("publicContract") {
            public_case_count += 1;
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
        if name == "position-dependent-attribute-queries-with-include-origin" {
            let included_bindings = response
                .attribute_queries
                .bindings
                .iter()
                .filter(|binding| binding.occurrence.name == "name")
                .collect::<Vec<_>>();
            assert_eq!(included_bindings.len(), 3, "{name}: included bindings");
            assert!(
                included_bindings
                    .iter()
                    .all(|binding| binding.source_id.as_deref() == Some("included:part.adoc")),
                "{name}: binding provenance"
            );
            let included_references = response
                .attribute_queries
                .references
                .iter()
                .filter(|reference| reference.name == "name")
                .collect::<Vec<_>>();
            assert_eq!(included_references.len(), 3, "{name}: included references");
            assert!(
                included_references
                    .iter()
                    .all(|reference| reference.source_id.as_deref() == Some("included:part.adoc")),
                "{name}: reference provenance"
            );
            let forward = response
                .attribute_queries
                .references
                .iter()
                .find(|reference| reference.name == "later")
                .expect("forward reference");
            assert_eq!(forward.binding_id, None, "{name}: forward binding");
            let header = response
                .attribute_queries
                .references
                .iter()
                .find(|reference| reference.name == "header-only")
                .expect("header reference");
            assert_eq!(
                header.effective_value.as_deref(),
                Some("root"),
                "{name}: header value"
            );
            let locked = response
                .attribute_queries
                .references
                .iter()
                .find(|reference| reference.name == "locked")
                .expect("locked reference");
            assert_eq!(locked.binding_id, None, "{name}: external binding");
            assert_eq!(
                locked.effective_value.as_deref(),
                Some("host"),
                "{name}: external value"
            );
            assert!(
                response
                    .attribute_queries
                    .bindings
                    .iter()
                    .all(|binding| binding.occurrence.name != "locked"
                        && binding.occurrence.name != "absent"),
                "{name}: rejected authored bindings"
            );
            let multiline = response
                .attribute_queries
                .bindings
                .iter()
                .find(|binding| binding.occurrence.name == "multi")
                .expect("multiline binding");
            let included_source = entry["preprocess"]["resources"]["part.adoc"]["source"]
                .as_str()
                .expect("included source");
            let second_line = &multiline.occurrence.value.lines[1];
            assert_eq!(
                &included_source[second_line.content_range.start as usize
                    ..second_line.content_range.end as usize],
                "second",
                "{name}: multiline line projection"
            );
        }
        assert_exclusive_expectation(entry, "expectedHtmlFile", "expectedHtml", name);
        if let Some(file) = entry["expectedHtmlFile"].as_str() {
            assert_eq!(
                response.html,
                fs::read_to_string(resolve(&fixtures, file)).expect("expected HTML"),
                "{name}"
            );
        }
        if let Some(expected) = entry["expectedHtml"].as_str() {
            assert_eq!(response.html, expected, "{name}: expectedHtml");
        }
        assert_exclusive_expectation(entry, "expectedAstFile", "expectedAst", name);
        if let Some(file) = entry["expectedAstFile"].as_str() {
            assert_eq!(
                response.ast,
                fs::read_to_string(resolve(&fixtures, file))
                    .expect("expected AST")
                    .trim_end(),
                "{name}: AST golden"
            );
        }
        if let Some(expected) = entry["expectedAst"].as_str() {
            assert_eq!(response.ast, expected, "{name}: expectedAst");
        }
        for (field, actual) in [
            (
                "expectedDiagnosticsFile",
                serde_json::to_value(&response.diagnostics).expect("diagnostics JSON"),
            ),
            (
                "expectedRenderDiagnosticsFile",
                serde_json::to_value(&response.render_diagnostics)
                    .expect("render diagnostics JSON"),
            ),
            (
                "expectedProjectionFile",
                serde_json::to_value(&response.projection).expect("projection JSON"),
            ),
            (
                "expectedSymbolsFile",
                serde_json::to_value(&response.symbols).expect("symbols JSON"),
            ),
        ] {
            let inline_field = field
                .strip_suffix("File")
                .expect("expected product file field");
            assert_exclusive_expectation(entry, field, inline_field, name);
            if let Some(file) = entry[field].as_str() {
                let expected: Value = serde_json::from_str(
                    &fs::read_to_string(resolve(&fixtures, file)).expect("expected JSON product"),
                )
                .expect("valid expected JSON product");
                assert_eq!(actual, expected, "{name}: {field}");
            }
            if let Some(expected) = entry.get(inline_field) {
                assert_eq!(actual, *expected, "{name}: {inline_field}");
            }
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
    for feature in contract["features"].as_array().expect("public features") {
        features.insert(feature.as_str().expect("public feature name"));
    }
    assert!(
        contract["implementationDetails"]
            .as_array()
            .is_some_and(|details| !details.is_empty()),
        "{name}: implementation details"
    );
    let stable = &contract["stableContract"];
    for assertion in stable["projectionAssertions"]
        .as_array()
        .expect("projection assertions")
    {
        let pointer = assertion["pointer"].as_str().expect("JSON pointer");
        assert_eq!(
            projection.pointer(pointer),
            Some(&assertion["value"]),
            "{name}: projection pointer {pointer}"
        );
    }
    for fragment in stable["htmlContains"].as_array().expect("HTML assertions") {
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

#[test]
fn browser_version_and_toolchains_have_separate_authorities() {
    let toolchains: Toolchains = serde_json::from_str(include_str!("../../../toolchains.json"))
        .expect("valid toolchain manifest");
    let browser: BrowserManifest =
        serde_json::from_str(include_str!("../../../web-worker/package.json"))
            .expect("valid Browser package manifest");
    assert_eq!(toolchains.schema_version, 1, "toolchain manifest schema");
    assert_eq!(browser.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(toolchains.rust_version, env!("CARGO_PKG_RUST_VERSION"));
    assert!(
        toolchains.node_version.split('.').count() == 3,
        "node version must be exact, found {}",
        toolchains.node_version,
    );
}

/// Builds the expected file contents of one case from the current implementation.
///
/// Regeneration and the cleanliness check share this function so a fixture can
/// never disagree with the command that writes it. Cases that expect an error,
/// and products a case does not name, contribute nothing.
///
fn expected_products(entry: &Value, fixtures: &Path) -> Vec<(PathBuf, String)> {
    if entry["expectedErrorCode"].is_string() {
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
fn shared_fixture_regeneration_is_clean() {
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
