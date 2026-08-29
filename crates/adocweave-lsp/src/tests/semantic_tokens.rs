use super::*;

#[test]
fn semantic_tokens_are_sorted_and_delta_encoded_from_the_analysis() {
    let mut service = Session::default();
    open_reference_workspace(&mut service);
    let tokens = service
        .semantic_tokens(&uri("file:///a.adoc"))
        .expect("semantic tokens")
        .expect("tokens");
    let value = serde_json::to_value(tokens).expect("serialize");
    let data = value["data"].as_array().expect("data");

    assert!(!data.is_empty());
    assert_eq!(data.len() % 5, 0);
    assert_eq!(data[0], 0);
}

#[test]
fn semantic_tokens_split_multiline_inline_ranges_at_crlf_boundaries() {
    for (encoding, first_length) in [("utf-8", 5), ("utf-16", 3)] {
        let mut service = Session::default();
        initialize(&mut service, &[encoding]);
        let document_uri = uri("file:///multiline.adoc");
        open(&mut service, document_uri.as_str(), 1, "``a😀\r\nb``");

        let tokens = service
            .semantic_tokens(&document_uri)
            .expect("semantic tokens")
            .expect("tokens");
        let value = serde_json::to_value(tokens).expect("serialize");

        assert_eq!(
            value["data"],
            json!([0, 2, first_length, 0, 0, 1, 0, 1, 0, 0])
        );
    }
}

#[test]
fn semantic_tokens_leave_syntactic_headings_to_editor_grammars() {
    let mut service = Session::default();
    initialize(&mut service, &["utf-8"]);
    let document_uri = uri("file:///heading.adoc");
    open(
        &mut service,
        document_uri.as_str(),
        1,
        "= Document\n\n== Section\n",
    );

    let tokens = service
        .semantic_tokens(&document_uri)
        .expect("semantic tokens")
        .expect("tokens");
    let value = serde_json::to_value(tokens).expect("serialize");

    assert_eq!(value["data"], json!([]));
}

#[test]
fn external_link_tokens_keep_exact_ranges_even_when_document_links_reject_the_url() {
    let source = "😀 https://example.com/path[Safe]\n\
javascript:alert(1)[Unsafe]\n\
https://example.com:99999[Invalid port]\n";
    for (encoding, expected_start) in [("utf-8", 5), ("utf-16", 3)] {
        let mut service = Session::default();
        initialize(&mut service, &[encoding]);
        let document = uri("file:///external-link-tokens.adoc");
        open(&mut service, document.as_str(), 1, source);

        let tokens = service
            .semantic_tokens(&document)
            .expect("semantic tokens")
            .expect("tokens");
        let value = serde_json::to_value(tokens).expect("serialize");

        assert_eq!(
            value["data"],
            json!([0, expected_start, 24, 0, 0, 1, 0, 19, 0, 0, 1, 0, 25, 0, 0]),
            "{encoding}"
        );
    }
}
