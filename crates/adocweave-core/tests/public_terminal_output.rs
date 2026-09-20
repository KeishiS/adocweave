use adocweave_core::output::terminal::{
    AmbiguousWidth, TerminalPolicy, TerminalRole, TerminalWidth, display_width, render,
};
use adocweave_core::{AnalysisOptions, Engine};

fn analyze(source: &str) -> adocweave_core::Analysis {
    Engine::new(AnalysisOptions::default())
        .analyze(source)
        .expect("analysis succeeds")
}

#[test]
fn public_terminal_output_lays_a_document_out_without_escape_sequences() {
    let analysis = analyze("= Guide\n\n== Intro\n\nSome text that is long enough to wrap.\n");
    let policy = TerminalPolicy {
        width: TerminalWidth::Columns(24),
        ..TerminalPolicy::default()
    };

    let rendered = render(analysis.document(), &policy);
    let text = rendered.document.to_plain_text();

    assert_eq!(rendered.package_version, adocweave_core::VERSION);
    assert!(rendered.diagnostics.is_empty());
    assert!(text.starts_with("Guide\n"));
    assert!(
        !text.contains('\u{1b}'),
        "the backend never writes escape sequences"
    );
    for line in &rendered.document.lines {
        assert!(line.display_width(AmbiguousWidth::Narrow) <= 24);
    }
}

#[test]
fn public_terminal_spans_carry_roles_a_host_maps_to_its_own_styling() {
    let analysis = analyze("= Guide\n\n*strong* text\n");

    let rendered = render(analysis.document(), &TerminalPolicy::default());
    let roles: Vec<TerminalRole> = rendered
        .document
        .lines
        .iter()
        .flat_map(|line| line.spans.iter().map(|span| span.style.role))
        .collect();

    assert!(roles.contains(&TerminalRole::DocumentTitle));
    assert!(
        rendered
            .document
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.text == "strong" && span.style.bold)
    );
}

#[test]
fn public_terminal_policy_defaults_to_eighty_columns_with_narrow_ambiguous_width() {
    let policy = TerminalPolicy::default();

    assert_eq!(policy.width, TerminalWidth::Columns(80));
    assert_eq!(policy.ambiguous_width, AmbiguousWidth::Narrow);
    assert!(policy.render_document_title);
}

#[test]
fn public_display_width_counts_terminal_columns() {
    assert_eq!(display_width("日本語", AmbiguousWidth::Narrow), 6);
    assert_eq!(display_width("text", AmbiguousWidth::Narrow), 4);
}

#[test]
fn public_terminal_output_names_what_a_terminal_cannot_show() {
    let analysis = analyze(
        "A claim. footnote:[The note.]\n\nimage::figure.png[A figure]\n\nSee https://example.com[the site].\n",
    );

    let rendered = render(analysis.document(), &TerminalPolicy::default());
    let text = rendered.document.to_plain_text();

    assert!(text.contains("[1]"), "{text}");
    assert!(text.contains("Footnotes"), "{text}");
    assert!(text.contains("[Image: A figure]"), "{text}");
    assert!(text.contains("the site (https://example.com)"), "{text}");
}
