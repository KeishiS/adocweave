use adocweave::NeverCancel;
use adocweave_wasm::{
    WasmActiveUrlPolicy, WasmAnalysisOptions, WasmAnalysisPreprocessInput, WasmAuthoredUrlPolicy,
    WasmCitationOutcome, WasmCitationSegment, WasmDiagnosticProfile, WasmDocumentMode, WasmError,
    WasmExternalLinkPolicy, WasmGeneratedBibliography, WasmGeneratedBibliographyEntry, WasmLimits,
    WasmOutputLimits, WasmPreprocessOptions, WasmPreprocessRequest, WasmPreprocessResponse,
    WasmRenderInputs, WasmRenderPolicy, WasmRequest, WasmResolvedCitation, WasmResolvedReference,
    WasmResolvedResource, WasmResource, WasmResourceCapabilities, WasmResourceOutcome,
    WasmRuleSettings, WasmSafeMode, WasmSourceLanguagePolicy, WasmSourceMapSegment,
    WasmSourceMapping, WasmStylesheet, WasmSyntaxMode, WasmSyntaxOptions,
    WasmUnknownSourceLanguage, WasmUnresolvedReferencePresentation, preprocess_request,
    process_request,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const CORPUS: &str = include_str!("../../../fixtures/protocol/request-corpus.json");

/// 受理と拒否を固定したrequest corpus。
///
/// wire型の正本はRustの定義です。この記録は与える入力と期待する結果を持ちます。
/// 以前は`protocol/public-api.json`が同じ形をもう一度宣言していましたが、Rustから
/// 生成したTypeScript宣言が公開契約になったため、二重の宣言をやめました。
fn corpus() -> Value {
    serde_json::from_str(CORPUS).expect("valid protocol corpus")
}

#[test]
fn latex_wire_value_remains_distinct_from_the_asciidoc_name() {
    let corpus = corpus();
    assert_eq!(corpus["enums"]["MathLanguage"], json!(["latex", "typst"]));
    assert_ne!(corpus["enums"]["MathLanguage"][0], "latexmath");
    assert_eq!(
        adocweave::semantic::MathLanguage::Latex.as_asciidoc_name(),
        "latexmath"
    );
}

fn base_request(corpus: &Value) -> Value {
    corpus["defaultRequest"].clone()
}

fn expanded_request(corpus: &Value) -> Value {
    let mut request = base_request(corpus);
    request["products"] = json!({
        "syntax": true,
        "canonicalAst": true,
        "html": true,
        "attributeOccurrences": true,
        "attributeQueries": true,
        "resourceQueries": true,
        "diagnostics": true,
        "symbols": true,
        "projection": true
    });
    request["analysisOptions"] = json!({
        "syntax": { "limits": {} },
        "diagnostics": {
            "rules": { "example": {} },
            "authoredUrls": {}
        }
    });
    request["preprocess"] = json!({
        "resources": {},
        "options": {}
    });
    request["renderPolicy"] = json!({
        "activeUrls": {},
        "externalLinks": {},
        "sourceLanguages": {},
        "roles": {},
        "resources": {},
        "mathLanguages": ["latex"],
        "stylesheets": [{ "kind": "inline", "css": "p {}" }]
    });
    request["renderInputs"] = json!({
        "references": [{
            "sourceStart": 0,
            "sourceEnd": 1,
            "outcome": { "status": "failed", "kind": "missing-target" }
        }],
        "resources": [{
            "sourceStart": 0,
            "sourceEnd": 1,
            "outcome": { "status": "failed", "kind": "missing" }
        }],
        "citations": [{
            "sourceStart": 0,
            "sourceEnd": 1,
            "outcome": { "status": "resolved", "segments": [{ "text": "(Smith 2024)" }] }
        }],
        "generatedBibliography": {
            "title": "References",
            "entries": [{
                "citationKey": "smith2024",
                "text": "Smith (2024)",
                "label": "Smith 2024",
                "number": 1
            }]
        }
    });
    request["outputLimits"] = json!({});
    request
}

fn set_pointer(document: &mut Value, pointer: &str, value: Value) {
    let (parent, field) = pointer.rsplit_once('/').expect("non-root JSON pointer");
    let parent = document
        .pointer_mut(parent)
        .unwrap_or_else(|| panic!("corpus path has no parent: {pointer}"));
    match parent {
        Value::Object(object) => {
            object.insert(field.to_owned(), value);
        }
        Value::Array(array) => {
            array[field.parse::<usize>().expect("array index")] = value;
        }
        _ => panic!("corpus path parent is not a container: {pointer}"),
    }
}

#[test]
fn default_request_uses_every_schema_default() {
    let corpus = corpus();
    let request: WasmRequest =
        serde_json::from_value(base_request(&corpus)).expect("default request is accepted");
    let mut without_source_id = base_request(&corpus);
    without_source_id
        .as_object_mut()
        .expect("request object")
        .remove("sourceId");
    assert_eq!(
        serde_json::from_value::<WasmRequest>(without_source_id)
            .expect("omitted source ID")
            .source_id,
        None
    );

    assert_eq!(request.products, Default::default());
    assert_eq!(
        serde_json::to_value(request.products).expect("product defaults"),
        corpus["browserProductDefault"]
    );
    assert_eq!(request.analysis_options, Default::default());
    assert_eq!(request.render_policy, Default::default());
    assert_eq!(request.output_limits, Default::default());
    let mut without_generated_bibliography = base_request(&corpus);
    without_generated_bibliography["renderInputs"] = json!({});
    let without_generated_bibliography: WasmRequest =
        serde_json::from_value(without_generated_bibliography)
            .expect("generated bibliography is optional");
    assert_eq!(
        without_generated_bibliography
            .render_inputs
            .generated_bibliography,
        None
    );
    // 省略したfieldへ入る既定値の一式。個々の値をもう一度宣言する代わりに、
    // 展開した結果を記録と突き合わせます。既定値が動けばここで気付きます。
    assert_eq!(
        serde_json::to_value(&request).expect("serializable default request"),
        corpus["defaultRequestExpansion"],
        "既定値の展開結果が記録と一致しません",
    );
    let expanded: WasmRequest =
        serde_json::from_value(expanded_request(&corpus)).expect("expanded request");
    serde_json::to_value(expanded).expect("serializable expanded request");
}

/// `defaultRequestExpansion`を現在の既定値から書き直します。
///
/// 既定値を意図して変えたときだけ実行し、差分を確認してからcommitします。
#[test]
#[ignore]
fn regenerate_default_request_expansion() {
    let mut corpus = corpus();
    let request: WasmRequest =
        serde_json::from_value(base_request(&corpus)).expect("default request is accepted");
    corpus["defaultRequestExpansion"] =
        serde_json::to_value(&request).expect("serializable default request");
    corpus["preprocessOptionsExpansion"] =
        serde_json::to_value(WasmPreprocessOptions::default()).expect("preprocess defaults");
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/request-corpus.json"
    );
    let mut text = serde_json::to_string_pretty(&corpus).expect("serializable corpus");
    text.push('\n');
    std::fs::write(path, text).expect("write corpus");
}

#[test]
fn public_request_wire_types_match_the_request_corpus_fixture() {
    fn assert_public<T>() {}

    assert_public::<WasmRequest>();
    assert_public::<WasmAnalysisOptions>();
    assert_public::<WasmSyntaxOptions>();
    assert_public::<WasmLimits>();
    assert_public::<WasmDiagnosticProfile>();
    assert_public::<WasmRuleSettings>();
    assert_public::<WasmAuthoredUrlPolicy>();
    assert_public::<WasmRenderPolicy>();
    assert_public::<WasmActiveUrlPolicy>();
    assert_public::<WasmExternalLinkPolicy>();
    assert_public::<WasmSourceLanguagePolicy>();
    assert_public::<WasmResourceCapabilities>();
    assert_public::<WasmOutputLimits>();
    assert_public::<WasmStylesheet>();

    let corpus = corpus();
    let request: WasmRequest =
        serde_json::from_value(expanded_request(&corpus)).expect("expanded fixture request");
    let serialized = serde_json::to_value(request).expect("serialized fixture request");
    // 受理した値をもう一度読み込めることを確かめます。serdeが往復できれば、
    // 公開している形と実装は一致しています。
    serde_json::from_value::<WasmRequest>(serialized).expect("round-tripped fixture request");
}

#[test]
fn generated_render_inputs_match_the_schema_safe_integer_boundary() {
    fn assert_public<T>() {}
    assert_public::<WasmRenderInputs>();
    assert_public::<WasmResolvedReference>();
    assert_public::<WasmResolvedResource>();
    assert_public::<WasmResolvedCitation>();
    assert_public::<WasmCitationOutcome>();
    assert_public::<WasmCitationSegment>();
    assert_public::<WasmGeneratedBibliography>();
    assert_public::<WasmGeneratedBibliographyEntry>();

    let outcome = |byte_length: Value| {
        serde_json::from_value::<WasmResourceOutcome>(json!({
            "status": "resolved",
            "href": "asset.png",
            "mediaType": "image/png",
            "byteLength": byte_length
        }))
    };
    assert!(outcome(json!(9_007_199_254_740_991_u64)).is_ok());
    for invalid in [json!(9_007_199_254_740_992_u64), json!(-1), json!(1.5)] {
        assert!(
            outcome(invalid.clone()).is_err(),
            "byteLength accepted {invalid}"
        );
    }
}

#[test]
fn generated_preprocess_wire_keeps_the_public_api_and_schema_defaults() {
    let corpus = corpus();
    let options = WasmPreprocessOptions::default();
    assert_eq!(options.safe_mode, WasmSafeMode::Secure);
    assert_eq!(
        serde_json::to_value(&options).expect("preprocess defaults"),
        corpus["preprocessOptionsExpansion"],
        "preprocessの既定値が記録と一致しません",
    );

    let resource = WasmResource {
        source_id: "file:///chapter.adoc".to_owned(),
        source: "= Chapter".to_owned(),
    };
    let analysis = WasmAnalysisPreprocessInput {
        resources: std::collections::BTreeMap::from([(
            "chapter.adoc".to_owned(),
            resource.clone(),
        )]),
        options: options.clone(),
    };
    let request = WasmPreprocessRequest {
        source_id: None,
        source: "include::chapter.adoc[]".to_owned(),
        resources: analysis.resources,
        options: analysis.options,
    };
    let serialized = serde_json::to_value(request).expect("public preprocess request");
    serde_json::from_value::<WasmPreprocessRequest>(serialized)
        .expect("round-tripped preprocess request");
    let response = WasmPreprocessResponse {
        source: "expanded".to_owned(),
        source_map: vec![WasmSourceMapSegment {
            output_start: 0,
            output_end: 8,
            source_id: Some("source".to_owned()),
            source_start: 1,
            source_end: 9,
            mapping: WasmSourceMapping::Identity,
        }],
    };
    serde_json::to_value(response).expect("public preprocess response");
    serde_json::to_value(WasmError {
        code: "invalid-request".to_owned(),
        message: "request is invalid".to_owned(),
    })
    .expect("public WASM error");

    const PREPROCESS_WIRE: &str = include_str!("../src/preprocess_wire.rs");
    for name in [
        "WasmPreprocessResponse",
        "WasmSourceMapSegment",
        "WasmSourceMapping",
        "WasmError",
    ] {
        assert!(PREPROCESS_WIRE.contains(name), "{name}");
    }
}

#[test]
fn preprocess_corpus_without_expansion_limits_uses_schema_defaults() {
    let corpus = corpus();
    let request: WasmPreprocessRequest =
        serde_json::from_value(corpus["preprocessRequest"].clone())
            .expect("schema 6 compatible preprocess request");
    let defaults = WasmPreprocessOptions::default();

    assert_eq!(
        request.options.max_attribute_expansion_depth,
        defaults.max_attribute_expansion_depth
    );
    assert_eq!(
        request.options.max_attribute_expansion_bytes,
        defaults.max_attribute_expansion_bytes
    );
}

#[test]
fn generated_request_enums_keep_the_public_api_and_schema_defaults() {
    let corpus = corpus();
    let cases = [
        (
            "SyntaxMode",
            serde_json::to_value(WasmSyntaxMode::Permissive).expect("syntax mode"),
            serde_json::to_value(WasmSyntaxMode::default()).expect("default syntax mode"),
        ),
        (
            "DocumentMode",
            serde_json::to_value(WasmDocumentMode::Fragment).expect("document mode"),
            serde_json::to_value(WasmDocumentMode::default()).expect("default document mode"),
        ),
        (
            "UnknownSourceLanguage",
            serde_json::to_value(WasmUnknownSourceLanguage::PreserveSanitized)
                .expect("unknown source language"),
            serde_json::to_value(WasmUnknownSourceLanguage::default())
                .expect("default unknown source language"),
        ),
        (
            "UnresolvedReferencePresentation",
            serde_json::to_value(WasmUnresolvedReferencePresentation::Target)
                .expect("unresolved reference presentation"),
            serde_json::to_value(WasmUnresolvedReferencePresentation::default())
                .expect("default unresolved reference presentation"),
        ),
    ];
    for (name, public_value, default_value) in cases {
        let expected = corpus["enums"][name][0].clone();
        assert_eq!(public_value, expected, "{name} public JSON value");
        assert_eq!(default_value, expected, "{name} default JSON value");
    }

    const REQUEST_ENUMS: &str = include_str!("../src/request_enums.rs");
    for name in [
        "WasmSyntaxMode",
        "WasmDocumentMode",
        "WasmUnknownSourceLanguage",
        "WasmUnresolvedReferencePresentation",
    ] {
        assert!(REQUEST_ENUMS.contains(&format!("pub enum {name}")));
    }
}

#[test]
fn request_accepts_every_input_enum_value_and_rejects_unknown_values() {
    let corpus = corpus();
    for case in corpus["enumCases"].as_array().expect("enum cases") {
        let name = case["enum"].as_str().expect("enum name");
        let path = case["path"].as_str().expect("enum path");
        let values = corpus["enums"][name].as_array().expect("schema enum");
        assert!(!values.is_empty(), "{name}");

        for value in values {
            let mut request = expanded_request(&corpus);
            apply_setup(&mut request, case);
            if let Some(template) = case
                .get("templates")
                .and_then(|templates| templates.get(value.as_str().expect("enum string")))
            {
                let parent = path.rsplit_once('/').expect("template path").0;
                set_pointer(&mut request, parent, template.clone());
            } else {
                set_pointer(&mut request, path, value.clone());
            }
            serde_json::from_value::<WasmRequest>(request)
                .unwrap_or_else(|error| panic!("{name} value {value} was rejected: {error}"));
        }

        let mut request = expanded_request(&corpus);
        apply_setup(&mut request, case);
        set_pointer(
            &mut request,
            path,
            Value::String("not-a-protocol-value".to_owned()),
        );
        assert!(
            serde_json::from_value::<WasmRequest>(request).is_err(),
            "{name} accepted an unknown value"
        );
    }
}

fn apply_setup(request: &mut Value, case: &Value) {
    for entry in case
        .get("setup")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        set_pointer(
            request,
            entry["path"].as_str().expect("setup path"),
            entry["value"].clone(),
        );
    }
}

#[test]
fn request_rejects_unknown_missing_and_removed_identity_fields() {
    let corpus = corpus();
    for case in corpus["unknownFieldCases"]
        .as_array()
        .expect("unknown field cases")
    {
        let path = case["path"].as_str().expect("unknown field path");
        let mut request = expanded_request(&corpus);
        set_pointer(&mut request, path, case["value"].clone());
        assert!(
            serde_json::from_value::<WasmRequest>(request).is_err(),
            "unknown field was accepted at {path}"
        );
    }

    for field in corpus["requiredRequestFields"]
        .as_array()
        .expect("required request fields")
    {
        let name = field.as_str().expect("field name");
        let mut request = base_request(&corpus);
        request
            .as_object_mut()
            .unwrap_or_else(|| panic!("request must be an object"))
            .remove(name);
        assert!(
            serde_json::from_value::<WasmRequest>(request).is_err(),
            "missing required field was accepted: {name}"
        );
    }

    for field in ["packageVersion", "version", "generation"] {
        let mut legacy = base_request(&corpus);
        legacy[field] = json!(1);
        assert!(
            serde_json::from_value::<WasmRequest>(legacy).is_err(),
            "removed request field was accepted: {field}"
        );
    }
}

#[test]
fn response_rejects_removed_identity_fields() {
    let corpus = corpus();
    let response = process_request(
        serde_json::from_value(base_request(&corpus)).expect("valid request"),
        &NeverCancel,
    )
    .expect("response");
    let value = serde_json::to_value(response).expect("serialized response");
    for field in ["packageVersion", "version", "generation"] {
        let mut legacy = value.clone();
        legacy[field] = json!(1);
        assert!(
            serde_json::from_value::<adocweave_wasm::WasmResponse>(legacy).is_err(),
            "removed response field was accepted: {field}"
        );
    }
}

/// requestのどのobjectでも、未知field、型違いおよびfieldの欠落を検査します。
///
/// 対象のfieldは、受理した値をもう一度serializeした結果から取ります。以前は
/// `protocol/public-api.json`が宣言するfield一覧を歩いていましたが、Rustが正本に
/// なった今は、実際に往復した値そのものが最も確かな一覧です。
#[test]
fn every_request_object_rejects_unknown_fields_and_wrong_types() {
    let corpus = corpus();
    let accepted: WasmRequest =
        serde_json::from_value(expanded_request(&corpus)).expect("expanded request");
    let serialized = serde_json::to_value(accepted).expect("serialized expanded request");

    for case in corpus["objectCases"].as_array().expect("object cases") {
        let name = case["object"].as_str().expect("object name");
        let path = case["path"].as_str().expect("object path");

        let mut unknown = expanded_request(&corpus);
        let object = unknown
            .pointer_mut(path)
            .unwrap_or_else(|| panic!("missing object path {path}"))
            .as_object_mut()
            .unwrap_or_else(|| panic!("{name} must be an object"));
        object.insert("unexpected".to_owned(), Value::Bool(true));
        assert!(
            serde_json::from_value::<WasmRequest>(unknown).is_err(),
            "{name} accepted an unknown field"
        );

        let fields = serialized
            .pointer(path)
            .unwrap_or_else(|| panic!("serialized object {path}"))
            .as_object()
            .unwrap_or_else(|| panic!("{name} must serialize to an object"));
        for (field_name, value) in fields {
            for invalid in wrong_typed_values(value) {
                let mut request = expanded_request(&corpus);
                request
                    .pointer_mut(path)
                    .expect("mutated object")
                    .as_object_mut()
                    .expect("mutated map")
                    .insert(field_name.clone(), invalid.clone());
                assert!(
                    serde_json::from_value::<WasmRequest>(request).is_err(),
                    "{name}.{field_name} accepted {invalid}"
                );
            }

            // 欠落は、必須なら拒否され、既定値を持つなら受理されます。どちらであるかは
            // Rustの定義が決めるため、ここでは「読み込めた場合も壊れない」ことを確かめます。
            let mut missing = expanded_request(&corpus);
            missing
                .pointer_mut(path)
                .expect("object path")
                .as_object_mut()
                .expect("object")
                .remove(field_name);
            if let Ok(request) = serde_json::from_value::<WasmRequest>(missing) {
                let filled = serde_json::to_value(request).expect("serialized default probe");
                assert!(
                    filled.pointer(path).is_some_and(|object| object
                        .as_object()
                        .is_some_and(|object| object.contains_key(field_name))),
                    "{name}.{field_name} disappeared after defaulting"
                );
            }
        }
    }
}

/// その値では受理されないはずのJSON。型ごとに、境界の外の値を返します。
fn wrong_typed_values(value: &Value) -> Vec<Value> {
    match value {
        Value::String(_) => vec![json!(false), json!(1)],
        Value::Bool(_) => vec![json!("invalid"), json!(1)],
        Value::Number(number) if number.is_u64() => {
            vec![json!(-1), json!(1.5), json!(4_294_967_296_u64), json!("1")]
        }
        Value::Number(_) => vec![json!("invalid")],
        Value::Array(_) => vec![json!("invalid"), json!(1)],
        Value::Object(_) => vec![json!("invalid"), json!(1)],
        Value::Null => vec![],
    }
}

#[test]
fn every_request_union_enforces_tags_fields_and_unknown_rejection() {
    let corpus = corpus();
    for case in corpus["unionCases"].as_array().expect("union cases") {
        let name = case["union"].as_str().expect("union name");
        let path = case["path"].as_str().expect("union path");
        let tag = case["tag"].as_str().expect("union tag");
        let variants = case["variants"].as_object().expect("variant templates");
        for (variant, template) in variants {
            let mut valid = expanded_request(&corpus);
            set_pointer(&mut valid, path, template.clone());
            let valid = serde_json::from_value::<WasmRequest>(valid)
                .unwrap_or_else(|error| panic!("{name}.{variant} was rejected: {error}"));
            let serialized = serde_json::to_value(valid).expect("serialized union probe");
            assert_eq!(
                serialized.pointer(path).expect("serialized union")[tag],
                *variant,
                "{name}.{variant} lost its tag"
            );

            let mut unknown = expanded_request(&corpus);
            let mut value = template.clone();
            value["unexpected"] = Value::Bool(true);
            set_pointer(&mut unknown, path, value);
            assert!(
                serde_json::from_value::<WasmRequest>(unknown).is_err(),
                "{name}.{variant} accepted an unknown field"
            );

            let mut missing_tag = expanded_request(&corpus);
            let mut value = template.clone();
            value.as_object_mut().expect("variant object").remove(tag);
            set_pointer(&mut missing_tag, path, value);
            assert!(
                serde_json::from_value::<WasmRequest>(missing_tag).is_err(),
                "{name}.{variant} accepted a missing tag"
            );

            let mut unknown_tag = expanded_request(&corpus);
            let mut value = template.clone();
            value[tag] = Value::String("not-a-variant".to_owned());
            set_pointer(&mut unknown_tag, path, value);
            assert!(
                serde_json::from_value::<WasmRequest>(unknown_tag).is_err(),
                "{name} accepted an unknown tag value"
            );
        }
    }
}

#[test]
fn response_and_projection_fields_match_the_schema() {
    let corpus = corpus();
    let mut value = base_request(&corpus);
    value["source"] = corpus["responseProbe"]["source"].clone();
    value["products"] = json!({
        "syntax": true,
        "canonicalAst": true,
        "html": true,
        "attributeOccurrences": true,
        "attributeQueries": true,
        "resourceQueries": true,
        "diagnostics": true,
        "symbols": true,
        "projection": true
    });
    let request: WasmRequest =
        serde_json::from_value(value.clone()).expect("response probe request");
    let response =
        serde_json::to_value(process_request(request, &NeverCancel).expect("response probe"))
            .expect("serializable response");

    for path in [
        "/attributeOccurrences/0",
        "/attributeQueries/bindings/0",
        "/attributeQueries/references/0",
        "/resourceQueries/0",
        "/diagnostics/0",
        "/projection/title",
        "/projection/targets/0",
        "/projection/structure/headings/0",
        "/projection/structure/toc/0",
        "/projection/sourceBlocks/0",
        "/projection/formulas/0",
        "/projection/orderedLists/0",
        "/projection/blockPresentations/0",
        "/projection/externalLinks/0",
        "/projection/referenceEdges/0",
        "/projection/searchableText/segments/0",
        "/projection/catalogs/footnotes/0",
        "/projection/catalogs/bibliography/0",
        "/projection/catalogs/index/0",
        "/symbols/0",
    ] {
        assert!(
            response.pointer(path).is_some(),
            "response probe must cover {path}"
        );
    }

    let edges = response["projection"]["referenceEdges"]
        .as_array()
        .expect("reference edges");
    let target_kinds = edges
        .iter()
        .map(|edge| edge["target"]["kind"].as_str().expect("target kind"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        target_kinds,
        BTreeSet::from(["document", "local", "scheme"]),
        "every ReferenceKey variant needs a non-empty wire witness"
    );
    value["renderInputs"] = json!({
        "references": [
            {
                "sourceStart": edges[0]["sourceRange"]["start"],
                "sourceEnd": edges[0]["sourceRange"]["end"],
                "outcome": {
                    "status": "resolved",
                    "href": "#first",
                    "notices": ["fallback"]
                }
            },
            {
                "sourceStart": edges[1]["sourceRange"]["start"],
                "sourceEnd": edges[1]["sourceRange"]["end"],
                "outcome": { "status": "failed", "kind": "missing-target" }
            }
        ]
    });
    let resolved = process_request(
        serde_json::from_value(value).expect("resolved response probe"),
        &NeverCancel,
    )
    .expect("resolved response");
    let resolved = serde_json::to_value(resolved).expect("serialized resolved response");
    assert_eq!(
        resolved["projection"]["referenceEdges"][0]["resolution"]["status"],
        "resolved"
    );
    assert_eq!(
        resolved["projection"]["referenceEdges"][0]["resolution"]["notices"],
        json!(["reference-resolution-fallback"])
    );
    assert_eq!(
        resolved["projection"]["referenceEdges"][1]["resolution"]["status"],
        "failed"
    );
}

#[test]
fn manpage_projection_is_a_non_empty_typed_schema_witness() {
    let corpus = corpus();
    let mut value = base_request(&corpus);
    value["source"] = json!(
        "= adocweave(1)\n:doctype: manpage\n\n== NAME\n\nadocweave - convert AsciiDoc safely\n"
    );
    value["products"] = json!({
        "syntax": false,
        "canonicalAst": false,
        "html": false,
        "attributeOccurrences": false,
        "attributeQueries": false,
        "resourceQueries": false,
        "diagnostics": false,
        "symbols": false,
        "projection": true
    });
    let response = process_request(
        serde_json::from_value(value).expect("manpage request"),
        &NeverCancel,
    )
    .expect("manpage response");
    let response = serde_json::to_value(response).expect("serialized manpage response");

    for field in [
        "name",
        "section",
        "purpose",
        "titleRange",
        "nameRange",
        "purposeRange",
    ] {
        assert!(
            response["projection"]["structure"]["manpage"]
                .get(field)
                .is_some(),
            "manpage witness must contain {field}"
        );
    }
}

#[test]
fn stable_typed_products_use_explicit_disabled_sentinels() {
    let corpus = corpus();
    let mut disabled = base_request(&corpus);
    disabled["products"] = json!({
        "syntax": false,
        "canonicalAst": false,
        "html": false,
        "attributeOccurrences": false,
        "attributeQueries": false,
        "resourceQueries": false,
        "diagnostics": false,
        "symbols": false,
        "projection": false
    });
    let request: WasmRequest = serde_json::from_value(disabled).expect("disabled request");
    let response = process_request(request, &NeverCancel).expect("default response");

    assert!(response.diagnostics.is_empty());
    assert!(response.render_diagnostics.is_empty());
    assert!(response.symbols.is_empty());
    assert!(response.projection.is_none());

    let mut enabled = base_request(&corpus);
    enabled["source"] = corpus["responseProbe"]["source"].clone();
    enabled["products"] = json!({
        "syntax": false,
        "canonicalAst": false,
        "html": true,
        "attributeOccurrences": false,
        "attributeQueries": false,
        "resourceQueries": false,
        "diagnostics": true,
        "symbols": true,
        "projection": true
    });
    let response = process_request(
        serde_json::from_value(enabled).expect("enabled request"),
        &NeverCancel,
    )
    .expect("enabled response");
    assert!(!response.diagnostics.is_empty());
    assert!(!response.symbols.is_empty());
    assert!(response.projection.is_some());
}

#[test]
fn preprocess_wire_contract_round_trips_and_rejects_drift() {
    let corpus = corpus();
    let value = corpus["preprocessRequest"].clone();
    let request: WasmPreprocessRequest =
        serde_json::from_value(value.clone()).expect("preprocess request");
    serde_json::to_value(&request).expect("serialized preprocess request");
    let defaults: WasmPreprocessRequest = serde_json::from_value(json!({
        "source": "text"
    }))
    .expect("default preprocess request");
    assert_eq!(defaults.source_id, None);
    assert_eq!(
        serde_json::to_value(&defaults.options).expect("preprocess defaults"),
        corpus["preprocessOptionsExpansion"],
    );
    let response = preprocess_request(request).expect("preprocess response");
    let response = serde_json::to_value(response).expect("serialized preprocess response");

    let mut legacy_request = value.clone();
    legacy_request["packageVersion"] = json!("0.46.2");
    assert!(serde_json::from_value::<WasmPreprocessRequest>(legacy_request).is_err());
    let mut legacy_response = response;
    legacy_response["packageVersion"] = json!("0.46.2");
    assert!(serde_json::from_value::<WasmPreprocessResponse>(legacy_response).is_err());

    for case in corpus["preprocessObjectCases"]
        .as_array()
        .expect("preprocess object cases")
    {
        let name = case["object"].as_str().expect("preprocess object name");
        let path = case["path"].as_str().expect("preprocess object path");
        let mut unknown = value.clone();
        unknown
            .pointer_mut(path)
            .expect("preprocess object")
            .as_object_mut()
            .expect("preprocess map")
            .insert("unexpected".to_owned(), Value::Bool(true));
        assert!(
            serde_json::from_value::<WasmPreprocessRequest>(unknown).is_err(),
            "preprocess unknown field at {path}"
        );

        let accepted: WasmPreprocessRequest =
            serde_json::from_value(value.clone()).expect("preprocess request");
        let serialized = serde_json::to_value(accepted).expect("serialized preprocess request");
        let fields = serialized
            .pointer(path)
            .unwrap_or_else(|| panic!("serialized preprocess object {path}"))
            .as_object()
            .unwrap_or_else(|| panic!("{name} must serialize to an object"));
        for (field_name, field_value) in fields {
            for invalid in wrong_typed_values(field_value) {
                let mut request = value.clone();
                request
                    .pointer_mut(path)
                    .expect("preprocess numeric object")
                    .as_object_mut()
                    .expect("preprocess numeric map")
                    .insert(field_name.clone(), invalid.clone());
                assert!(
                    serde_json::from_value::<WasmPreprocessRequest>(request).is_err(),
                    "{name}.{field_name} accepted {invalid}"
                );
            }
            let mut missing = value.clone();
            missing
                .pointer_mut(path)
                .expect("preprocess object path")
                .as_object_mut()
                .expect("preprocess object")
                .remove(field_name);
            if let Ok(request) = serde_json::from_value::<WasmPreprocessRequest>(missing) {
                let filled = serde_json::to_value(request).expect("serialized preprocess default");
                assert!(
                    filled.pointer(path).is_some_and(|object| object
                        .as_object()
                        .is_some_and(|object| object.contains_key(field_name))),
                    "{name}.{field_name} disappeared after defaulting"
                );
            }
        }
    }
    for mode in corpus["enums"]["SafeMode"].as_array().expect("safe modes") {
        let mut request = value.clone();
        request["options"]["safeMode"] = mode.clone();
        serde_json::from_value::<WasmPreprocessRequest>(request)
            .unwrap_or_else(|error| panic!("safe mode {mode}: {error}"));
    }
    let mut invalid_mode = value;
    invalid_mode["options"]["safeMode"] = Value::String("invalid".to_owned());
    assert!(serde_json::from_value::<WasmPreprocessRequest>(invalid_mode).is_err());
}
