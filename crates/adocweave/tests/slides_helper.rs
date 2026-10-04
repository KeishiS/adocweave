#[allow(dead_code, unused_imports)]
#[path = "../src/slides/helper/mod.rs"]
mod helper;

#[test]
fn protocol_schema_rejects_unknown_fields() {
    let json = r#"{"schemaVersion":1,"eqnums":"none","scopes":{"body":{"equations":[],"citations":[]},"notes":{"equations":[],"citations":[]}},"extra":true}"#;
    assert!(serde_json::from_str::<helper::protocol::Request>(json).is_err());
}

use adocweave_core::{
    AnalysisOptions, CancellationToken, Engine, NeverCancel,
    output::projection::formulas,
    text::{TextRange, TextSize},
};
use helper::{
    HostResult, Prepared, Selection,
    protocol::{self, Scope, Severity},
};
use std::collections::BTreeSet;

fn range(start: usize, end: usize) -> TextRange {
    TextRange::new(TextSize::new(start).unwrap(), TextSize::new(end).unwrap()).unwrap()
}
fn fixture() -> Prepared {
    let request: protocol::Request =
        serde_json::from_str(include_str!("fixtures/slides-helper/request.json")).unwrap();
    let mut sources = helper::SourceMap::new();
    let mut offset = 0;
    for (scope, input) in request.scopes.iter() {
        for key in input
            .equations
            .iter()
            .map(|e| &e.key)
            .chain(input.citations.iter().map(|c| &c.key))
        {
            sources.insert((scope, key.clone()), range(offset, offset + 1));
            offset += 2;
        }
    }
    Prepared {
        request,
        sources,
        diagnostics: Vec::new(),
    }
}
fn response() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/slides-helper/response.json")).unwrap()
}
fn validate(json: serde_json::Value) -> HostResult<helper::ValidatedResults> {
    helper::validate_response(
        &fixture(),
        serde_json::from_value(json).map_err(|e| helper::HostError::protocol(e.to_string()))?,
        0,
        &BTreeSet::new(),
    )
}

#[test]
fn actual_mathjax_and_citeproc_fixture_is_accepted() {
    let results = validate(response()).unwrap();
    assert_eq!(results.inputs.body.math().len(), 6);
    assert_eq!(results.inputs.notes.math().len(), 2);
    assert_eq!(results.inputs.body.rich_citations().len(), 2);
    assert_eq!(results.inputs.notes.rich_citations().len(), 1);
    assert!(results.diagnostics.is_empty());
}

#[test]
fn every_result_key_is_accounted_for() {
    let mut missing = response();
    missing["scopes"]["body"]["equations"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert!(validate(missing).is_err());
    let mut duplicate = response();
    duplicate["scopes"]["body"]["equations"][1] =
        duplicate["scopes"]["body"]["equations"][0].clone();
    assert!(validate(duplicate).is_err());
    let mut unknown = response();
    unknown["scopes"]["notes"]["citations"][0]["key"] = "invented".into();
    assert!(validate(unknown).is_err());
    let mut extra = response();
    extra["scopes"]["body"]["equations"][0]["html"] = "<script>".into();
    assert!(validate(extra).is_err());
}

#[test]
fn failed_results_require_matching_error_diagnostics_and_exit_status() {
    let mut json = response();
    json["scopes"]["body"]["equations"][0] = serde_json::json!({"key":"math1","status":"failed"});
    let parsed: protocol::Response = serde_json::from_value(json.clone()).unwrap();
    assert!(helper::validate_response(&fixture(), parsed, 1, &BTreeSet::new()).is_err());
    json["diagnostics"] = serde_json::json!([{"scope":"body","key":"math1","severity":"error","code":"math-failed","message":"invalid TeX"}]);
    let parsed: protocol::Response = serde_json::from_value(json.clone()).unwrap();
    let accepted = helper::validate_response(&fixture(), parsed, 1, &BTreeSet::new()).unwrap();
    assert_eq!(
        accepted.diagnostics[0].range,
        fixture()
            .sources
            .get(&(Scope::Body, "math1".into()))
            .copied()
    );
    assert_eq!(accepted.diagnostics[0].severity, Severity::Error);
    assert!(
        helper::validate_response(
            &fixture(),
            serde_json::from_value(json).unwrap(),
            0,
            &BTreeSet::new()
        )
        .is_err()
    );
}

#[test]
fn bibliography_ids_and_math_targets_are_request_relative() {
    let mut uncited = response();
    uncited["scopes"]["body"]["bibliography"][0]["id"] = "unknown".into();
    assert!(validate(uncited).is_err());
    let mut duplicate = response();
    duplicate["scopes"]["body"]["bibliography"][1] =
        duplicate["scopes"]["body"]["bibliography"][0].clone();
    assert!(validate(duplicate).is_err());
    let mut dangling = response();
    let svg = dangling["scopes"]["body"]["equations"][0]["svg"]
        .as_str()
        .unwrap()
        .replace("#body-math2-i8", "#body-absent-i8");
    dangling["scopes"]["body"]["equations"][0]["svg"] = svg.into();
    assert!(validate(dangling).is_err());
    let parsed: protocol::Response = serde_json::from_value(response()).unwrap();
    assert!(
        helper::validate_response(
            &fixture(),
            parsed,
            0,
            &BTreeSet::from(["body-math1-i0".into()])
        )
        .is_err()
    );
}

#[test]
fn public_preparation_drops_notes_and_note_only_library_items() {
    let analysis=Engine::new(AnalysisOptions::default()).analyze("= Deck\n:eqnums:\n\nBody latexmath:[x] cite:[public].\n\nNotes latexmath:[y] cite:[private].\n").unwrap();
    let equations = formulas(&analysis);
    let citations = analysis.citations();
    let body = Selection {
        equations: vec![equations[0].source_range],
        citations: vec![citations[0].range],
        footnotes: Vec::new(),
    };
    let notes = Selection {
        equations: vec![equations[1].source_range],
        citations: vec![citations[1].range],
        footnotes: Vec::new(),
    };
    let csl = protocol::Csl {
        items: vec![
            serde_json::json!({"id":"public"}),
            serde_json::json!({"id":"private","title":"private title"}),
        ],
        style: "style".into(),
        locale: "locale".into(),
    };
    let prepared = helper::prepare(&analysis, &body, &notes, false, None, Some(csl)).unwrap();
    assert!(prepared.request.scopes.notes.equations.is_empty());
    assert!(prepared.request.scopes.notes.citations.is_empty());
    assert!(matches!(prepared.request.eqnums, protocol::Eqnums::Ams));
    let encoded = serde_json::to_string(&prepared.request).unwrap();
    assert!(!encoded.contains("private"));
    assert!(!encoded.contains("sourceRange"));
    assert!(
        prepared
            .sources
            .keys()
            .all(|(scope, _)| *scope == Scope::Body)
    );
}

#[test]
fn unsupported_notation_is_diagnosed_at_the_source_and_explicit_latex_still_runs() {
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze(
            "= Deck\n:stem: strange\n\nEquation stem:[x] and asciimath:[y] and latexmath:[z].\n",
        )
        .unwrap();
    let equations = formulas(&analysis);
    let selection = Selection {
        equations: equations.iter().rev().map(|e| e.source_range).collect(),
        citations: Vec::new(),
        footnotes: Vec::new(),
    };
    let prepared = helper::prepare(
        &analysis,
        &selection,
        &Selection::default(),
        true,
        None,
        None,
    )
    .unwrap();
    assert_eq!(prepared.diagnostics.len(), 2);
    assert!(prepared.diagnostics[0].message.contains("stem=strange"));
    assert!(!prepared.diagnostics[1].message.contains("stem="));
    assert_eq!(
        prepared.diagnostics[0].range,
        Some(equations[0].source_range)
    );
    assert_eq!(prepared.request.scopes.body.equations[0].tex, "z");
}

#[test]
fn numbering_defaults_and_invalid_values_are_explicit() {
    for (attribute, expected) in [
        ("", "none"),
        (":eqnums:\n", "ams"),
        (":eqnums: AMS\n", "ams"),
        (":eqnums: all\n", "all"),
        (":eqnums: none\n", "none"),
    ] {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(&format!("= Deck\n{attribute}\nlatexmath:[x]\n"))
            .unwrap();
        let prepared = helper::prepare(
            &analysis,
            &Selection::default(),
            &Selection::default(),
            false,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(prepared.request.eqnums).unwrap(),
            expected
        );
    }
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("= Deck\n:eqnums: broken\n\nlatexmath:[x]\n")
        .unwrap();
    assert_eq!(
        helper::prepare(
            &analysis,
            &Selection::default(),
            &Selection::default(),
            false,
            None,
            None
        )
        .unwrap_err()
        .code,
        "slides-eqnums-invalid"
    );
}

#[test]
fn request_limits_and_cross_category_duplicate_keys_are_rejected() {
    let mut prepared = fixture();
    prepared.request.scopes.body.citations[0].key = "math1".into();
    assert!(prepared.request.validate().is_err());
    let mut prepared = fixture();
    prepared.request.scopes.body.equations[0].tex = "x".repeat(16 * 1024 + 1);
    assert!(prepared.request.validate().is_err());
    let mut prepared = fixture();
    prepared.request.macros.as_mut().unwrap()[0].arguments = Some(10);
    assert!(prepared.request.validate().is_err());
}

#[tokio::test]
async fn no_math_or_citations_requires_no_executable() {
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("Hello.\n")
        .unwrap();
    let prepared = helper::prepare(
        &analysis,
        &Selection::default(),
        &Selection::default(),
        false,
        None,
        None,
    )
    .unwrap();
    let results = helper::execute(
        &prepared,
        Some(std::path::Path::new("/missing/helper")),
        &NeverCancel,
        helper::ProcessLimits::default(),
        &BTreeSet::new(),
    )
    .await
    .unwrap();
    assert!(results.inputs.body.math().is_empty());
}

#[cfg(unix)]
fn fake_helper(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("helper");
    std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    (directory, executable)
}

#[tokio::test]
async fn helper_waiting_for_stdin_eof_receives_one_complete_json_request() {
    let directory = tempfile::Builder::new()
        .prefix("slides helper with spaces ")
        .tempdir()
        .unwrap();
    let path = directory.path().join("bin.mjs");
    std::fs::write(
        &path,
        r#"import { readFileSync } from 'node:fs';
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const request = JSON.parse(Buffer.concat(chunks).toString('utf8'));
if (request.schemaVersion !== 1 || !request.scopes.body.equations.length) process.exit(2);
process.stdout.write(readFileSync(new URL('./response.json', import.meta.url)));
"#,
    )
    .unwrap();
    std::fs::write(
        directory.path().join("response.json"),
        include_str!("fixtures/slides-helper/response.json"),
    )
    .unwrap();
    let result = helper::execute(
        &fixture(),
        Some(&path),
        &NeverCancel,
        helper::ProcessLimits {
            timeout: std::time::Duration::from_secs(5),
            ..Default::default()
        },
        &BTreeSet::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.inputs.body.math().len(), 6);
    assert_eq!(result.inputs.notes.math().len(), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn stdout_logs_extra_json_and_stream_limits_are_rejected() {
    for (script, code) in [
        ("printf 'log\\n{}'", "slides-helper-protocol"),
        ("printf '{}{}'", "slides-helper-protocol"),
        ("printf 'xxxxxxxxxxxxxxxx'", "slides-helper-output-limit"),
    ] {
        let (_directory, path) = fake_helper(script);
        let limits = helper::ProcessLimits {
            output_bytes: if code.ends_with("limit") { 4 } else { 1024 },
            ..Default::default()
        };
        let error = helper::execute(
            &fixture(),
            Some(&path),
            &NeverCancel,
            limits,
            &BTreeSet::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, code);
    }
    let (_directory, path) = fake_helper("printf 'should not run'");
    let error = helper::execute(
        &fixture(),
        Some(&path),
        &NeverCancel,
        helper::ProcessLimits {
            input_bytes: 1,
            ..Default::default()
        },
        &BTreeSet::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "slides-helper-input-limit");
}
#[cfg(unix)]
#[tokio::test]
async fn cancellation_and_timeout_reap_the_child() {
    for cancelled in [false, true] {
        let (_directory, path) = fake_helper("exec sleep 20");
        let pid_file = _directory.path().join("child.pid");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec sleep 20\n",
                pid_file.display()
            ),
        )
        .unwrap();

        let token = CancellationToken::default();
        let error = if cancelled {
            let cancellation = async {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                token.cancel();
            };
            let prepared = fixture();
            let reserved = BTreeSet::new();
            let (result, ()) = tokio::join!(
                helper::execute(
                    &prepared,
                    Some(&path),
                    &token,
                    helper::ProcessLimits::default(),
                    &reserved
                ),
                cancellation
            );
            result.unwrap_err()
        } else {
            helper::execute(
                &fixture(),
                Some(&path),
                &token,
                helper::ProcessLimits {
                    timeout: std::time::Duration::from_millis(200),
                    ..Default::default()
                },
                &BTreeSet::new(),
            )
            .await
            .unwrap_err()
        };
        let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        // A reaped direct child no longer exists, including after timeout/cancellation.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        assert_eq!(
            error.code,
            if cancelled {
                "slides-helper-cancelled"
            } else {
                "slides-helper-timeout"
            }
        );
    }
}

#[test]
fn actual_node_fatal_response_retains_the_original_diagnostic() {
    let parsed =
        serde_json::from_str(include_str!("fixtures/slides-helper/fatal-response.json")).unwrap();
    let error = helper::validate_response(&fixture(), parsed, 1, &BTreeSet::new()).unwrap_err();
    assert_eq!(error.code, "slides-helper-failed");
    assert!(
        error
            .message
            .contains("invalid-request: request.schemaVersion is required.")
    );
}

#[test]
fn helper_sources_remain_projectable_to_included_files() {
    use adocweave_core::{
        SourceId,
        preprocess::{
            EffectiveProcessingOptions, ExpandedRange, PreprocessInputs, PreprocessOptions,
            ResourceDocument, ResourceSnapshot,
        },
    };
    let mut snapshot = ResourceSnapshot::default();
    snapshot.insert(
        "part.adoc",
        ResourceDocument {
            source_id: SourceId::new("part"),
            source: "latexmath:[x < y] cite:[paper].\n".into(),
        },
    );
    let processed =
        EffectiveProcessingOptions::new(AnalysisOptions::default(), PreprocessOptions::default())
            .unwrap()
            .preprocess_and_analyze(
                "= Deck\n\ninclude::part.adoc[]\n",
                &snapshot,
                PreprocessInputs::default(),
            )
            .unwrap();
    let selection = Selection {
        equations: formulas(&processed.analysis)
            .iter()
            .map(|f| f.source_range)
            .collect(),
        citations: processed
            .analysis
            .citations()
            .iter()
            .map(|c| c.range)
            .collect(),
        footnotes: Vec::new(),
    };
    let csl = protocol::Csl {
        items: vec![serde_json::json!({"id":"paper"})],
        style: "style".into(),
        locale: "locale".into(),
    };
    let prepared = helper::prepare(
        &processed.analysis,
        &selection,
        &Selection::default(),
        false,
        None,
        Some(csl),
    )
    .unwrap();
    for range in prepared.sources.values() {
        let origins = processed
            .document
            .origins_for_range(ExpandedRange::new(*range));
        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0].source_id, Some(SourceId::new("part")));
    }
}

#[test]
fn representative_ams_mathjax_shapes_match_the_finite_profile() {
    let request: protocol::Request =
        serde_json::from_str(include_str!("fixtures/slides-helper/shapes-request.json")).unwrap();
    let mut sources = helper::SourceMap::new();
    let mut offset = 0;
    for (scope, input) in request.scopes.iter() {
        for equation in &input.equations {
            sources.insert((scope, equation.key.clone()), range(offset, offset + 1));
            offset += 2;
        }
    }
    let prepared = Prepared {
        request,
        sources,
        diagnostics: Vec::new(),
    };
    let response =
        serde_json::from_str(include_str!("fixtures/slides-helper/shapes-response.json")).unwrap();
    let results = helper::validate_response(&prepared, response, 0, &BTreeSet::new()).unwrap();
    assert_eq!(results.inputs.body.math().len(), 5);
    assert!(results.inputs.notes.math().is_empty());
}

#[test]
fn diagnostics_require_explicit_nullable_scope_and_key() {
    for diagnostic in [
        serde_json::json!({"severity":"error","code":"failed","message":"failure"}),
        serde_json::json!({"scope":null,"severity":"error","code":"failed","message":"failure"}),
        serde_json::json!({"key":null,"severity":"error","code":"failed","message":"failure"}),
    ] {
        assert!(serde_json::from_value::<protocol::Diagnostic>(diagnostic).is_err());
    }
    assert!(serde_json::from_value::<protocol::Diagnostic>(serde_json::json!({"scope":null,"key":null,"severity":"error","code":"failed","message":"failure"})).is_ok());
}

#[test]
fn notices_are_required_bounded_text_and_match_the_included_dependencies() {
    let json = response();
    let parsed: protocol::Response = serde_json::from_value(json.clone()).unwrap();
    assert!(parsed.notices.math.is_some());
    assert!(parsed.notices.citations.is_some());
    let results = validate(json.clone()).unwrap();
    assert!(results.notices.math.is_some());
    let mut missing = json.clone();
    missing.as_object_mut().unwrap().remove("notices");
    assert!(validate(missing).is_err());
    let mut extra = json.clone();
    extra["notices"]["math"]["url"] = "https://example.org".into();
    assert!(validate(extra).is_err());
    let mut mismatched = json.clone();
    mismatched["notices"]["math"] = serde_json::Value::Null;
    assert!(validate(mismatched).is_err());
    let mut oversized = json.clone();
    oversized["notices"]["math"]["fontLicense"] = "x".repeat(64 * 1024 + 1).into();
    assert!(validate(oversized).is_err());
    let mut control = json.clone();
    control["notices"]["citations"]["license"] = "\0".into();
    assert!(validate(control).is_err());
    let mut total = json;
    for name in [
        "fontAttribution",
        "fontLicense",
        "lpplLicense",
        "mathjaxLicense",
    ] {
        total["notices"]["math"][name] = "x".repeat(64 * 1024).into();
    }
    assert!(validate(total).is_err());
}

#[test]
fn footnote_stem_uses_the_definition_attribute_position_and_explicit_notation_wins() {
    use adocweave_core::semantic::{MathLanguage, StandardMacroKind};
    for (attribute, language) in [
        ("", MathLanguage::AsciiMath),
        (":stem:\n", MathLanguage::AsciiMath),
        (":stem: latex\n", MathLanguage::Latex),
        (":stem: tex\n", MathLanguage::Latex),
        (":stem: latexmath\n", MathLanguage::Latex),
        (":stem: asciimath\n", MathLanguage::AsciiMath),
        (":stem: unknown\n", MathLanguage::AsciiMath),
    ] {
        let analysis = Engine::new(AnalysisOptions::default()).analyze(&format!("= Deck\n{attribute}\nFirst footnote:shared[stem:[x] latexmath:[y]].\n\n:stem: asciimath\n\nAgain footnote:shared[].\n")).unwrap();
        let selection = Selection {
            footnotes: analysis
                .macros()
                .iter()
                .filter(|node| node.kind == StandardMacroKind::Footnote)
                .map(|node| node.range)
                .collect(),
            ..Default::default()
        };
        let selected =
            helper::selected_content(&analysis, &selection, &Selection::default(), false).unwrap();
        assert_eq!(selected.body.equations.len(), 2);
        assert_eq!(selected.body.equations[0].language, language, "{attribute}");
        assert_eq!(selected.body.equations[1].language, MathLanguage::Latex);
        let prepared = helper::prepare(
            &analysis,
            &selection,
            &Selection::default(),
            false,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            prepared.diagnostics.len(),
            usize::from(language != MathLanguage::Latex)
        );
        if attribute.contains("unknown") {
            assert!(prepared.diagnostics[0].message.contains("stem=unknown"));
        }
        assert_eq!(
            prepared.request.scopes.body.equations.last().unwrap().tex,
            "y"
        );
    }
}

#[test]
fn footnote_helper_sources_keep_include_origins_and_shared_scopes_are_the_only_reuse() {
    use adocweave_core::{
        SourceId,
        preprocess::{
            EffectiveProcessingOptions, ExpandedRange, PreprocessInputs, PreprocessOptions,
            ResourceDocument, ResourceSnapshot,
        },
        semantic::StandardMacroKind,
    };
    let mut snapshot = ResourceSnapshot::default();
    snapshot.insert(
        "part.adoc",
        ResourceDocument {
            source_id: SourceId::new("part"),
            source: ":stem: tex\n\nDefinition footnote:shared[stem:[x] cite:[paper]].\n".into(),
        },
    );
    let processed =
        EffectiveProcessingOptions::new(AnalysisOptions::default(), PreprocessOptions::default())
            .unwrap()
            .preprocess_and_analyze(
                "= Deck\n\ninclude::part.adoc[]\n\n:stem!:\n\nNotes footnote:shared[].\n",
                &snapshot,
                PreprocessInputs::default(),
            )
            .unwrap();
    let occurrences = processed
        .analysis
        .macros()
        .iter()
        .filter(|node| node.kind == StandardMacroKind::Footnote)
        .map(|node| node.range)
        .collect::<Vec<_>>();
    let body = Selection {
        footnotes: vec![occurrences[0]],
        ..Default::default()
    };
    let notes = Selection {
        footnotes: vec![occurrences[1]],
        ..Default::default()
    };
    let csl = protocol::Csl {
        items: vec![serde_json::json!({"id":"paper"})],
        style: "style".into(),
        locale: "locale".into(),
    };
    let prepared =
        helper::prepare(&processed.analysis, &body, &notes, true, None, Some(csl)).unwrap();
    assert_eq!(prepared.request.scopes.body.equations.len(), 1);
    assert_eq!(prepared.request.scopes.notes.equations.len(), 1);
    assert_eq!(
        prepared.sources[&(Scope::Body, "m0".into())],
        prepared.sources[&(Scope::Notes, "m0".into())]
    );
    for source in prepared.sources.values() {
        let origins = processed
            .document
            .origins_for_range(ExpandedRange::new(*source));
        assert_eq!(origins[0].source_id, Some(SourceId::new("part")));
    }
    assert!(helper::selected_content(&processed.analysis, &body, &body, true).is_err());
    let public = helper::prepare(&processed.analysis, &body, &notes, false, None, None).unwrap();
    assert!(public.request.scopes.notes.equations.is_empty());
    assert!(public.request.scopes.notes.citations.is_empty());
}

#[test]
fn footnote_stem_unsetting_does_not_change_an_earlier_shared_definition() {
    use adocweave_core::semantic::{MathLanguage, StandardMacroKind};
    let analysis = Engine::new(AnalysisOptions::default()).analyze("= Deck\n:stem: tex\n\nFirst footnote:shared[stem:[x]].\n\n:stem!:\n\nSecond footnote:[stem:[y]].\n\n:stem: unknown\n\nThird footnote:[stem:[z]].\n\nAgain footnote:shared[].\n").unwrap();
    let selection = Selection {
        footnotes: analysis
            .macros()
            .iter()
            .filter(|node| node.kind == StandardMacroKind::Footnote)
            .map(|node| node.range)
            .collect(),
        ..Default::default()
    };
    let selected =
        helper::selected_content(&analysis, &selection, &Selection::default(), false).unwrap();
    assert_eq!(
        selected
            .body
            .equations
            .iter()
            .map(|formula| formula.language)
            .collect::<Vec<_>>(),
        [
            MathLanguage::Latex,
            MathLanguage::AsciiMath,
            MathLanguage::AsciiMath
        ]
    );
    let prepared = helper::prepare(
        &analysis,
        &selection,
        &Selection::default(),
        false,
        None,
        None,
    )
    .unwrap();
    assert_eq!(prepared.request.scopes.body.equations.len(), 1);
    assert_eq!(prepared.diagnostics.len(), 2);
    assert!(!prepared.diagnostics[0].message.contains("stem="));
    assert!(prepared.diagnostics[1].message.contains("stem=unknown"));
}

#[test]
fn absence_of_external_csl_never_sends_manual_citations_to_the_helper() {
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("cite:[manual].\n\n[bibliography]\n== References\n\n* [[[manual]]] Entry.\n")
        .unwrap();
    let selection = Selection {
        citations: analysis
            .citations()
            .iter()
            .map(|citation| citation.range)
            .collect(),
        ..Default::default()
    };
    let prepared = helper::prepare(
        &analysis,
        &selection,
        &Selection::default(),
        false,
        None,
        None,
    )
    .unwrap();
    assert!(prepared.request.scopes.body.citations.is_empty());
    let output = helper::execute_sync(
        &prepared,
        Some(std::path::Path::new("/missing/helper")),
        &NeverCancel,
        Default::default(),
        &BTreeSet::new(),
    )
    .unwrap();
    assert!(output.inputs.body.rich_citations().is_empty());
}
