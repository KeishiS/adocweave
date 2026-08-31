use super::*;

#[test]
fn document_symbols_choose_the_empty_shape_from_client_capabilities() {
    let project = TestProject::new();
    let document_uri = project.document("not-analyzed.adoc", "");
    let mut hierarchical = Session::default();
    initialize(&mut hierarchical, &["utf-16"]);
    assert!(matches!(
        hierarchical
            .document_symbols(&document_uri)
            .expect("hierarchical symbols")
            .expect("response"),
        lsp::DocumentSymbolResponse::Nested(symbols) if symbols.is_empty()
    ));

    let mut flat = Session::default();
    flat.initialize(&typed(json!({
        "processId": null,
        "rootUri": null,
        "capabilities": {}
    })));
    assert!(matches!(
        flat.document_symbols(&document_uri)
            .expect("flat symbols")
            .expect("response"),
        lsp::DocumentSymbolResponse::Flat(symbols) if symbols.is_empty()
    ));
}

#[test]
fn hover_and_completion_use_the_same_analysis_snapshot() {
    let project = TestProject::new();
    let mut service = Session::default();
    let document_uri = project.open(
        &mut service,
        "features.adoc",
        1,
        "= 題名😀\n\n[source, ru]\n----\ncode\n----\n",
    );
    let hover = service
        .hover(&document_uri, lsp::Position::new(0, 4))
        .expect("hover")
        .expect("value");
    let hover = serde_json::to_value(hover).expect("serialize");
    assert!(
        hover["contents"]["value"]
            .as_str()
            .expect("text")
            .contains("Generated ID")
    );

    let completion = service
        .completion(&document_uri, lsp::Position::new(2, 11))
        .expect("completion")
        .expect("response");
    let completion = serde_json::to_value(completion).expect("serialize");
    assert_eq!(completion, json!([{"label": "rust", "kind": 12}]));
}

#[test]
fn hover_and_completion_cover_attributes_references_links_and_math() {
    let project = TestProject::new();
    let mut service = Session::default();
    let document_uri = project.open(
        &mut service,
        "rich-features.adoc",
        1,
        "= Title\n:name: value\n\n[[part]]\n== Part\n\nhttps://example.com[Site] <<part>> stem:[x+y]\n",
    );
    for (position, expected) in [
        (lsp::Position::new(1, 2), "document attribute"),
        (lsp::Position::new(3, 3), "reference target"),
        (lsp::Position::new(6, 3), "external link"),
        (lsp::Position::new(6, 29), "cross reference"),
        (lsp::Position::new(6, 43), "LaTeX formula"),
    ] {
        let hover = service
            .hover(&document_uri, position)
            .expect("hover")
            .expect("value");
        let value = serde_json::to_value(hover).expect("serialize");
        assert!(
            value["contents"]["value"]
                .as_str()
                .expect("hover text")
                .contains(expected),
            "expected {expected} at {position:?}: {value}"
        );
    }
    let completion = service
        .completion(&document_uri, lsp::Position::new(6, 31))
        .expect("completion")
        .expect("response");
    let value = serde_json::to_value(completion).expect("serialize");
    assert!(
        value
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["label"] == "part")
    );
}

#[test]
fn hover_selects_the_token_starting_at_an_adjacent_range_boundary() {
    let project = TestProject::new();
    let mut service = Session::default();
    let document = project.open(
        &mut service,
        "adjacent-attributes.adoc",
        1,
        ":first: one\n:second: two\n\n{first}{second}\n",
    );
    let hover = service
        .hover(&document, lsp::Position::new(3, 7))
        .expect("hover")
        .expect("second attribute hover");
    let value = serde_json::to_value(hover).expect("serialize");

    assert!(
        value["contents"]["value"]
            .as_str()
            .expect("hover text")
            .contains("Value: `two`"),
        "{value}"
    );
}

#[test]
fn attribute_features_follow_the_binding_visible_at_the_cursor() {
    let source = "\
:name: first

{name}

:name: second

{name}

:name!:

{name}
";
    let project = TestProject::new();
    let mut service = Session::default();
    let document = project.open(&mut service, "attributes.adoc", 1, source);

    for (line, expected) in [(2, "Value: `first`"), (6, "Value: `second`"), (10, "unset")] {
        let hover = service
            .hover(&document, lsp::Position::new(line, 2))
            .expect("hover")
            .expect("attribute hover");
        let value = serde_json::to_value(hover).expect("serialize");
        assert!(
            value["contents"]["value"]
                .as_str()
                .expect("hover text")
                .contains(expected),
            "{value}"
        );
    }

    let first_definition = service
        .definition(&document, lsp::Position::new(2, 2))
        .expect("definition")
        .expect("first binding");
    let second_definition = service
        .definition(&document, lsp::Position::new(6, 2))
        .expect("definition")
        .expect("second binding");
    let lsp::GotoDefinitionResponse::Scalar(first_definition) = first_definition else {
        panic!("scalar definition");
    };
    let lsp::GotoDefinitionResponse::Scalar(second_definition) = second_definition else {
        panic!("scalar definition");
    };
    assert_eq!(first_definition.range.start, lsp::Position::new(0, 1));
    assert_eq!(second_definition.range.start, lsp::Position::new(4, 1));

    let first_references = service
        .references(&document, lsp::Position::new(0, 2), true)
        .expect("references")
        .expect("locations");
    assert_eq!(
        first_references
            .iter()
            .map(|location| location.range.start)
            .collect::<Vec<_>>(),
        [lsp::Position::new(0, 1), lsp::Position::new(2, 1)]
    );

    let before_unset = service
        .completion(&document, lsp::Position::new(6, 3))
        .expect("completion")
        .expect("items");
    assert!(
        serde_json::to_value(before_unset)
            .expect("serialize")
            .as_array()
            .expect("array")
            .iter()
            .any(|item| item["label"] == "name" && item["detail"] == "second")
    );
    let after_unset = service
        .completion(&document, lsp::Position::new(10, 3))
        .expect("completion")
        .expect("items");
    assert!(
        serde_json::to_value(after_unset)
            .expect("serialize")
            .as_array()
            .expect("array")
            .iter()
            .all(|item| item["label"] != "name")
    );
}

#[test]
fn attribute_definition_and_references_project_through_includes() {
    let fixture = IncludeFixture::new(
        "include::part.adoc[]\n\n😀 {shared}\n",
        ":shared: included\n",
    );
    let mut service = Session::default();
    open(
        &mut service,
        fixture.include_uri.as_str(),
        1,
        ":shared: included\n",
    );
    open(
        &mut service,
        fixture.root_uri.as_str(),
        1,
        "include::part.adoc[]\n\n😀 {shared}\n",
    );
    let root = fixture.root_uri.clone();
    let part = fixture.include_uri.clone();

    let hover = service
        .hover(&root, lsp::Position::new(2, 4))
        .expect("hover")
        .expect("attribute hover");
    assert!(
        serde_json::to_value(hover).expect("serialize")["contents"]["value"]
            .as_str()
            .expect("hover text")
            .contains("Value: `included`")
    );
    let definition = service
        .definition(&root, lsp::Position::new(2, 4))
        .expect("definition")
        .expect("included binding");
    let lsp::GotoDefinitionResponse::Scalar(definition) = definition else {
        panic!("scalar definition");
    };
    assert_eq!(definition.uri, part);
    assert_eq!(definition.range.start, lsp::Position::new(0, 1));

    let references = service
        .references(&part, lsp::Position::new(0, 2), true)
        .expect("references")
        .expect("locations");
    assert!(
        references.iter().any(|location| {
            location.uri == root && location.range.start == lsp::Position::new(2, 4)
        }),
        "{references:#?}"
    );

    assert!(
        change(
            &mut service,
            fixture.include_uri.as_str(),
            2,
            json!([{"text": ":shared: changed\n"}])
        )
        .expect("included edit")
    );
    let updated = service
        .hover(&root, lsp::Position::new(2, 4))
        .expect("updated hover")
        .expect("attribute hover");
    assert!(
        serde_json::to_value(updated).expect("serialize")["contents"]["value"]
            .as_str()
            .expect("hover text")
            .contains("Value: `changed`")
    );

    let mut restarted = Session::default();
    open(
        &mut restarted,
        fixture.include_uri.as_str(),
        2,
        ":shared: changed\n",
    );
    open(
        &mut restarted,
        fixture.root_uri.as_str(),
        1,
        "include::part.adoc[]\n\n😀 {shared}\n",
    );
    let restarted_hover = restarted
        .hover(&root, lsp::Position::new(2, 4))
        .expect("restarted hover")
        .expect("attribute hover");
    assert!(
        serde_json::to_value(restarted_hover).expect("serialize")["contents"]["value"]
            .as_str()
            .expect("hover text")
            .contains("Value: `changed`")
    );
}

#[test]
fn attribute_features_use_the_negotiated_utf8_position_encoding() {
    let project = TestProject::new();
    let mut service = Session::default();
    initialize(&mut service, &["utf-8"]);
    let document = project.open(
        &mut service,
        "utf8-attribute.adoc",
        1,
        ":name: 値\n\n😀 {name}\n",
    );
    let hover = service
        .hover(&document, lsp::Position::new(2, 6))
        .expect("hover")
        .expect("attribute hover");
    assert!(
        serde_json::to_value(hover).expect("serialize")["contents"]["value"]
            .as_str()
            .expect("hover text")
            .contains("Value: `値`")
    );
    let definition = service
        .definition(&document, lsp::Position::new(2, 6))
        .expect("definition")
        .expect("binding");
    let lsp::GotoDefinitionResponse::Scalar(definition) = definition else {
        panic!("scalar definition");
    };
    assert_eq!(definition.range.start, lsp::Position::new(0, 1));
}

#[test]
fn hover_and_completion_cover_common_block_metadata() {
    let project = TestProject::new();
    let mut service = Session::default();
    let document_uri = project.open(
        &mut service,
        "metadata.adoc",
        1,
        ".Visible\n[#item.lead%collapsible,cols=2]\nParagraph\n",
    );
    for (position, expected) in [
        (lsp::Position::new(0, 2), "block title"),
        (lsp::Position::new(1, 3), "reference target"),
        (lsp::Position::new(1, 8), "block role"),
        (lsp::Position::new(1, 14), "block option"),
        (lsp::Position::new(1, 28), "cols"),
    ] {
        let hover = service
            .hover(&document_uri, position)
            .expect("hover")
            .expect("value");
        let value = serde_json::to_value(hover).expect("serialize");
        assert!(
            value["contents"]["value"]
                .as_str()
                .expect("hover text")
                .contains(expected),
            "expected {expected} at {position:?}: {value}"
        );
    }

    let completion = service
        .completion(&document_uri, lsp::Position::new(1, 28))
        .expect("completion")
        .expect("response");
    let value = serde_json::to_value(completion).expect("serialize");
    assert!(
        value
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["label"] == "subs")
    );
    assert!(
        value
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["label"] == "frame")
    );
}

#[test]
fn hover_and_completion_cover_semantic_block_presentations() {
    let project = TestProject::new();
    let mut service = Session::default();
    let document_uri = project.open(
        &mut service,
        "semantic-blocks.adoc",
        1,
        "NOTE: text\n\n[quote,Author,Work]\n____\nquoted\n____\n",
    );
    let note = service
        .hover(&document_uri, lsp::Position::new(0, 1))
        .expect("hover")
        .expect("note hover");
    assert!(
        serde_json::to_value(note).expect("serialize")["contents"]["value"]
            .as_str()
            .expect("text")
            .contains("NOTE admonition")
    );
    let quote = service
        .hover(&document_uri, lsp::Position::new(2, 2))
        .expect("hover")
        .expect("quote hover");
    assert!(
        serde_json::to_value(quote).expect("serialize")["contents"]["value"]
            .as_str()
            .expect("text")
            .contains("quote block")
    );
    let completion = service
        .completion(&document_uri, lsp::Position::new(2, 2))
        .expect("completion")
        .expect("response");
    assert!(
        serde_json::to_value(completion)
            .expect("serialize")
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["label"] == "NOTE")
    );
}

#[test]
fn hover_uses_document_catalogs_for_footnotes_bibliography_and_index() {
    let project = TestProject::new();
    let mut service = Session::default();
    let source = "footnote:n[note] footnote:n[] bibanchor:ref[] indexterm:[Rust,Ownership]";
    let document_uri = project.open(&mut service, "catalogs.adoc", 1, source);
    for (character, expected) in [
        (2, "footnote 1"),
        (23, "footnote 1"),
        (37, "bibliography entry"),
        (55, "Rust > Ownership"),
    ] {
        let hover = service
            .hover(&document_uri, lsp::Position::new(0, character))
            .expect("hover")
            .expect("value");
        let value = serde_json::to_value(hover).expect("serialize");
        assert!(
            value["contents"]["value"]
                .as_str()
                .expect("hover text")
                .contains(expected),
            "expected {expected}: {value}"
        );
    }
}

#[test]
fn bibliography_targets_support_hover_definition_and_references() {
    let project = TestProject::new();
    let mut service = Session::default();
    let source = "= References\n\n[bibliography]\n== Sources\n\n* bibanchor:ref[] Entry\n\nSee <<ref,Entry>> and <<ref>>.\n";
    let document_uri = project.open(&mut service, "bibliography.adoc", 1, source);

    let hover = service
        .hover(&document_uri, lsp::Position::new(5, 9))
        .expect("hover")
        .expect("value");
    assert!(
        serde_json::to_value(hover).expect("serialize")["contents"]["value"]
            .as_str()
            .expect("hover text")
            .contains("bibliography entry")
    );

    let definition = service
        .definition(&document_uri, lsp::Position::new(7, 6))
        .expect("definition")
        .expect("value");
    let definition = serde_json::to_value(definition).expect("serialize");
    assert_eq!(definition["uri"], document_uri.as_str());
    assert_eq!(definition["range"]["start"]["line"], 5);

    let references = service
        .references(&document_uri, lsp::Position::new(5, 13), true)
        .expect("references")
        .expect("locations");
    assert_eq!(references.len(), 3);
}
