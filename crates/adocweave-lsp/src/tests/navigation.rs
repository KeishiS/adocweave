use super::*;

#[test]
fn definition_resolves_local_and_open_document_targets() {
    let mut service = Session::default();
    open_reference_workspace(&mut service);
    let document_uri = uri("file:///a.adoc");

    let local = service
        .definition(&document_uri, lsp::Position::new(3, 7))
        .expect("definition")
        .expect("local definition");
    let local = serde_json::to_value(local).expect("serialize");
    assert_eq!(local["uri"], "file:///a.adoc");
    assert_eq!(local["range"]["start"]["line"], 1);

    let external = service
        .definition(&document_uri, lsp::Position::new(3, 28))
        .expect("definition")
        .expect("document definition");
    let external = serde_json::to_value(external).expect("serialize");
    assert_eq!(external["uri"], "file:///b.adoc");
    assert_eq!(external["range"]["start"]["line"], 1);
}

#[test]
fn references_use_one_workspace_identity_for_local_and_document_xrefs() {
    let mut service = Session::default();
    open_reference_workspace(&mut service);
    let locations = service
        .references(&uri("file:///a.adoc"), lsp::Position::new(0, 3), true)
        .expect("references")
        .expect("locations");
    let values = serde_json::to_value(locations).expect("serialize");

    assert_eq!(values.as_array().expect("locations").len(), 3);
    assert!(
        values
            .as_array()
            .expect("locations")
            .iter()
            .any(|location| location["uri"] == "file:///b.adoc")
    );
}

#[test]
fn references_report_unicode_ranges_in_utf8_and_utf16() {
    let source = "[[節😀]]\n== 見出し\n\n<<節😀>>\n";
    for (encoding, expected_end) in [(PositionEncoding::Utf8, 9), (PositionEncoding::Utf16, 5)] {
        let mut service = Session::default();
        service.position_encoding = encoding;
        open(&mut service, "file:///unicode-ref.adoc", 1, source);
        let references = service
            .references(
                &uri("file:///unicode-ref.adoc"),
                lsp::Position::new(0, 2),
                false,
            )
            .expect("references")
            .expect("locations");

        assert_eq!(references.len(), 1);
        assert_eq!(references[0].range.start.character, 2);
        assert_eq!(references[0].range.end.character, expected_end);
    }
}

#[test]
fn document_links_keep_safe_urls_and_xrefs_separate_but_navigable() {
    let mut service = Session::default();
    open_reference_workspace(&mut service);
    let links = service
        .document_links(&uri("file:///a.adoc"))
        .expect("document links")
        .expect("links");
    let values = serde_json::to_value(links).expect("serialize");
    let targets = values
        .as_array()
        .expect("links")
        .iter()
        .map(|link| link["target"].as_str().expect("target"))
        .collect::<Vec<_>>();

    assert_eq!(targets.len(), 3);
    assert!(targets.contains(&"https://example.com/"));
    assert!(targets.contains(&"file:///a.adoc#target"));
    assert!(targets.contains(&"file:///b.adoc#other"));
}

#[test]
fn document_links_keep_exact_target_ranges_and_reject_unsafe_or_invalid_urls() {
    let source = "😀 https://example.com/path[Safe]\n\
javascript:alert(1)[Unsafe]\n\
https://example.com:99999[Invalid port]\n";
    for (encoding, expected_start) in [("utf-8", 5), ("utf-16", 3)] {
        let mut service = Session::default();
        initialize(&mut service, &[encoding]);
        let document = uri("file:///external-links.adoc");
        open(&mut service, document.as_str(), 1, source);

        let links = service
            .document_links(&document)
            .expect("document links")
            .expect("links");

        assert_eq!(links.len(), 1, "{encoding}");
        let link = &links[0];
        assert_eq!(
            link.target.as_ref().map(lsp::Url::as_str),
            Some("https://example.com/path"),
            "{encoding}"
        );
        assert_eq!(
            link.range,
            lsp::Range::new(
                lsp::Position::new(0, expected_start),
                lsp::Position::new(0, expected_start + 24),
            ),
            "{encoding}"
        );
    }
}

#[test]
fn rename_uses_open_document_analyses() {
    let mut incomplete = Session::default();
    open(
        &mut incomplete,
        "file:///a.adoc",
        1,
        "[[target]]\n== A\n\nSee <<target>>.\n",
    );
    let local_edit = incomplete
        .rename(&uri("file:///a.adoc"), lsp::Position::new(0, 3), "renamed")
        .expect("rename")
        .expect("workspace edit");
    let local_changes = local_edit.changes.expect("changes");
    let edits = &local_changes[&uri("file:///a.adoc")];
    assert_eq!(edits.len(), 2);
    assert!(edits.contains(&lsp::TextEdit::new(
        lsp::Range::new(lsp::Position::new(0, 2), lsp::Position::new(0, 8)),
        "renamed".to_owned(),
    )));
    assert!(edits.contains(&lsp::TextEdit::new(
        lsp::Range::new(lsp::Position::new(3, 6), lsp::Position::new(3, 12)),
        "renamed".to_owned(),
    )));
    assert!(
        edits
            .iter()
            .all(|edit| edit.range.start.line != 1 && edit.range.end.line != 1),
        "rename must not replace the block an anchor targets",
    );
}

#[test]
fn rename_preserves_cross_document_reference_locators() {
    let mut service = Session::default();
    open_reference_workspace(&mut service);

    let edit = service
        .rename(&uri("file:///a.adoc"), lsp::Position::new(0, 3), "renamed")
        .expect("rename")
        .expect("workspace edit");
    let changes = edit.changes.expect("changes");
    let external_edits = &changes[&uri("file:///b.adoc")];
    assert!(
        external_edits.contains(&lsp::TextEdit::new(
            lsp::Range::new(lsp::Position::new(3, 12), lsp::Position::new(3, 18)),
            "renamed".to_owned(),
        )),
        "unexpected external edits: {external_edits:?}",
    );
    assert!(
        external_edits
            .iter()
            .all(|edit| edit.range.start.character >= 12),
        "rename must preserve the document locator and # separator",
    );
}

#[test]
fn rename_refuses_expanded_reference_destinations() {
    let mut service = Session::default();
    initialize(&mut service, &["utf-16"]);
    open(&mut service, "file:///a.adoc", 1, "[[target]]\n== A\n");
    open(
        &mut service,
        "file:///b.adoc",
        1,
        ":destination: a.adoc#target\n\nxref:{destination}[A]\n",
    );

    let edit = service
        .rename(&uri("file:///a.adoc"), lsp::Position::new(0, 3), "renamed")
        .expect("rename query");

    assert_eq!(
        edit, None,
        "an expanded destination has no editable anchor token"
    );
    assert!(
        service
            .prepare_rename(&uri("file:///a.adoc"), lsp::Position::new(0, 3))
            .expect("prepare rename")
            .is_none(),
        "prepareRename and rename must apply the same safety check",
    );
}

#[test]
fn rename_rejects_an_existing_target_id() {
    let mut service = Session::default();
    open(
        &mut service,
        "file:///a.adoc",
        1,
        "[[one]]\n== One\n\n[[two]]\n== Two\n\n<<one>>\n",
    );

    assert!(
        service
            .rename(&uri("file:///a.adoc"), lsp::Position::new(0, 3), "two")
            .expect("rename")
            .is_none(),
        "rename must not create a duplicate target ID",
    );
}

#[test]
fn rename_accepts_only_authored_anchor_ids() {
    let mut service = Session::default();
    initialize(&mut service, &["utf-16"]);
    open(
        &mut service,
        "file:///a.adoc",
        1,
        "== Generated Heading\n\n[[explicit]]\n== Explicit Heading\n\nanchor:inline[]\n",
    );
    let document = uri("file:///a.adoc");

    assert!(
        service
            .prepare_rename(&document, lsp::Position::new(0, 5))
            .expect("generated heading")
            .is_none(),
    );
    assert!(
        service
            .rename(&document, lsp::Position::new(0, 5), "renamed")
            .expect("generated heading rename")
            .is_none(),
    );

    let explicit = service
        .prepare_rename(&document, lsp::Position::new(2, 3))
        .expect("explicit anchor")
        .expect("explicit anchor is renameable");
    assert_eq!(
        explicit,
        lsp::PrepareRenameResponse::RangeWithPlaceholder {
            range: lsp::Range::new(lsp::Position::new(2, 2), lsp::Position::new(2, 10)),
            placeholder: "explicit".to_owned(),
        }
    );

    let inline = service
        .prepare_rename(&document, lsp::Position::new(5, 9))
        .expect("inline anchor")
        .expect("inline anchor is renameable");
    assert_eq!(
        inline,
        lsp::PrepareRenameResponse::RangeWithPlaceholder {
            range: lsp::Range::new(lsp::Position::new(5, 7), lsp::Position::new(5, 13)),
            placeholder: "inline".to_owned(),
        }
    );

    assert!(
        service
            .rename(&document, lsp::Position::new(2, 3), "a&b")
            .expect("invalid anchor rename")
            .is_none(),
        "rename must reject every identifier the parser rejects",
    );
}

#[test]
fn prepare_rename_answers_only_where_rename_produces_an_edit() {
    let mut service = Session::default();
    initialize(&mut service, &["utf-16"]);
    open(
        &mut service,
        "file:///a.adoc",
        1,
        "[[target]]\n== A\n\nplain text\n",
    );
    let document = uri("file:///a.adoc");

    let anchor = lsp::Position::new(0, 3);
    let response = service
        .prepare_rename(&document, anchor)
        .expect("prepare rename")
        .expect("renameable position");
    assert_eq!(
        response,
        lsp::PrepareRenameResponse::RangeWithPlaceholder {
            range: lsp::Range::new(lsp::Position::new(0, 2), lsp::Position::new(0, 8)),
            placeholder: "target".to_owned(),
        }
    );
    assert!(
        service
            .rename(&document, anchor, "renamed")
            .expect("rename")
            .is_some(),
        "a position prepareRename accepts must produce an edit",
    );

    // Body text holds no anchor. Before prepareRename the editor started a
    // rename here and received nothing.
    let prose = lsp::Position::new(3, 2);
    assert!(
        service
            .prepare_rename(&document, prose)
            .expect("prepare rename")
            .is_none(),
    );
    assert!(
        service
            .rename(&document, prose, "renamed")
            .expect("rename")
            .is_none(),
        "a position prepareRename rejects must produce no edit",
    );
}

#[test]
fn prepare_rename_is_silent_for_clients_that_do_not_declare_support() {
    let mut service = Session::default();
    let params = typed(json!({
        "processId": null,
        "rootUri": null,
        "capabilities": {
            "general": {"positionEncodings": ["utf-16"]},
            "textDocument": {"rename": {"dynamicRegistration": false}}
        }
    }));
    let result = initialize_with_params(&mut service, params);
    assert_eq!(
        result.capabilities.rename_provider,
        Some(lsp::OneOf::Left(true)),
        "a client without prepare support keeps the plain declaration",
    );

    open(&mut service, "file:///a.adoc", 1, "[[target]]\n== A\n");
    assert!(
        service
            .prepare_rename(&uri("file:///a.adoc"), lsp::Position::new(0, 3))
            .expect("prepare rename")
            .is_none(),
    );
    assert!(
        service
            .rename(&uri("file:///a.adoc"), lsp::Position::new(0, 3), "renamed")
            .expect("rename")
            .is_some(),
        "rename itself keeps working without prepare support",
    );
}
