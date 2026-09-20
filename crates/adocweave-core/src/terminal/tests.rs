use super::layout::{Canvas, wrap_spans};
use super::{
    AmbiguousWidth, IndentPolicy, TerminalDocument, TerminalPolicy, TerminalRole, TerminalSpan,
    TerminalStyle, TerminalWidth, display_width, render,
};

fn analyze(source: &str) -> crate::core::Analysis {
    crate::core::Engine::new(crate::core::AnalysisOptions::default())
        .analyze(source)
        .expect("analysis")
}

fn policy(columns: u16) -> TerminalPolicy {
    TerminalPolicy {
        width: TerminalWidth::Columns(columns),
        ..TerminalPolicy::default()
    }
}

/// The rendered document as plain text, which is what a reader sees when the
/// host drops every style.
fn plain(source: &str, columns: u16) -> String {
    render(analyze(source).document(), &policy(columns))
        .document
        .to_plain_text()
}

fn document(source: &str, columns: u16) -> TerminalDocument {
    render(analyze(source).document(), &policy(columns)).document
}

#[test]
fn paragraph_lines_join_and_wrap_to_the_requested_width() {
    assert_eq!(
        plain("one two three\nfour five six seven eight nine", 20),
        "one two three four\nfive six seven\neight nine\n"
    );
}

/// The paragraph line join follows the same rule as the HTML backend: a line
/// break becomes a space, except between two characters of a script written
/// without spaces between words.
#[test]
fn joined_lines_follow_the_html_rule_for_scripts_without_word_spaces() {
    assert_eq!(
        plain("これは日本語の段落です。\n行を折り返しています。", 60),
        "これは日本語の段落です。行を折り返しています。\n"
    );
    assert_eq!(
        plain("日本語とEnglish\nmixed の場合。", 60),
        "日本語とEnglish mixed の場合。\n"
    );
}

#[test]
fn text_without_spaces_wraps_between_characters() {
    assert_eq!(
        plain("これは日本語の段落です。折り返しを確認します。", 20),
        "これは日本語の段落で\nす。折り返しを確認し\nます。\n"
    );
}

/// A line may not begin with a mark that belongs to the character before it.
#[test]
fn a_line_never_begins_with_a_closing_mark() {
    let rendered = plain("あいうえおかきくけこ。さしすせそ", 20);

    for line in rendered.lines() {
        assert!(
            !line.starts_with('。'),
            "line begins with a closing mark: {line}"
        );
    }
    assert_eq!(rendered, "あいうえおかきくけ\nこ。さしすせそ\n");
}

#[test]
fn a_word_longer_than_the_width_is_broken_where_the_width_runs_out() {
    assert_eq!(
        plain(
            "see https://example.com/a/very/long/path/that/never/ends",
            20
        ),
        "see\nhttps://example.com/\na/very/long/path/tha\nt/never/ends\n"
    );
}

#[test]
fn every_wrapped_line_stays_within_the_requested_width() {
    let source = "= Title\n\n日本語のtextとLatin wordsが混ざった段落です。\nもう一行あります。\n\n== Section\n\nAnother paragraph with several words in it.\n";
    let rendered = document(source, 24);

    for line in &rendered.lines {
        assert!(
            line.display_width(AmbiguousWidth::Narrow) <= 24,
            "line is too wide: {:?}",
            line.text()
        );
    }
}

#[test]
fn rendering_the_same_document_twice_gives_the_same_result() {
    let source = "= Title\n\nIntro paragraph.\n\n== Section\n\nBody text.\n";
    let analysis = analyze(source);

    assert_eq!(
        render(analysis.document(), &policy(40)),
        render(analysis.document(), &policy(40))
    );
}

#[test]
fn the_document_title_is_underlined_and_followed_by_its_header_metadata() {
    assert_eq!(
        plain(
            "= Terminal Output\nAuthor Name <author@example.com>\nv1.2, 2026-09-20\n\nBody.\n",
            40
        ),
        "Terminal Output\n═══════════════\nAuthor Name <author@example.com>\nv1.2 — 2026-09-20\n\nBody.\n"
    );
}

#[test]
fn the_document_title_is_left_out_when_the_host_asks_for_no_title() {
    let source = "= Terminal Output\n\nBody.\n";
    let rendered = render(
        analyze(source).document(),
        &TerminalPolicy {
            render_document_title: false,
            ..policy(40)
        },
    );

    assert_eq!(rendered.document.to_plain_text(), "Body.\n");
}

/// Depth is shown by indentation and by a rule under the outermost levels, so
/// the hierarchy is readable with no styling at all.
#[test]
fn heading_depth_is_readable_without_styling() {
    assert_eq!(
        plain(
            "== First\n\ntext\n\n=== Nested\n\ntext\n\n==== Deeper\n\ntext\n",
            40
        ),
        "First\n─────\n\ntext\n\n  Nested\n\ntext\n\n    Deeper\n\ntext\n"
    );
}

#[test]
fn a_numbered_heading_carries_its_section_number() {
    assert_eq!(
        plain(":sectnums:\n\n== First\n\n=== Nested\n", 40),
        "1. First\n────────\n\n  1.1. Nested\n"
    );
}

/// A marker without a space is not a heading. It reads as the text it is, the
/// same way the HTML backend shows it.
#[test]
fn a_malformed_heading_reads_as_ordinary_text() {
    assert_eq!(plain("==Not a heading\n", 40), "Not a heading\n");
}

/// A literal paragraph is never reflowed, and the relative indentation the
/// author wrote is kept under the indentation of the block.
#[test]
fn a_literal_paragraph_keeps_its_lines_and_is_indented() {
    assert_eq!(
        plain("  literal line one\n    literal line two\n", 40),
        "     literal line one\n       literal line two\n"
    );
}

#[test]
fn a_thematic_break_fills_the_content_width() {
    assert_eq!(
        plain("text\n\n'''\n\nmore\n", 10),
        "text\n\n──────────\n\nmore\n"
    );
}

#[test]
fn blocks_are_separated_by_one_blank_line_and_the_page_has_no_leading_blank() {
    assert_eq!(plain("one\n\ntwo\n\nthree\n", 40), "one\n\ntwo\n\nthree\n");
}

#[test]
fn inline_styles_carry_roles_and_emphasis_rather_than_markers() {
    let rendered = document("*strong* and _emphasis_ and `code`\n", 60);
    let spans = &rendered.lines[0].spans;

    assert_eq!(rendered.lines[0].text(), "strong and emphasis and code");
    assert!(
        spans
            .iter()
            .any(|span| span.text == "strong" && span.style.bold)
    );
    assert!(
        spans
            .iter()
            .any(|span| span.text == "emphasis" && span.style.italic)
    );
    assert!(
        spans
            .iter()
            .any(|span| span.text == "code" && span.style.role == TerminalRole::Monospace)
    );
}

/// A terminal cannot raise or lower text, so the marker the author typed stays.
#[test]
fn raised_and_lowered_text_keeps_the_marker_the_author_typed() {
    assert_eq!(plain("x^2^ and H~2~O\n", 40), "x^2 and H~2O\n");
}

#[test]
fn a_heading_span_carries_its_level_and_the_title_span_its_own_role() {
    let rendered = document("= Title\n\n== Section\n", 40);
    let roles: Vec<TerminalRole> = rendered
        .lines
        .iter()
        .flat_map(|line| line.spans.iter().map(|span| span.style.role))
        .collect();

    assert!(roles.contains(&TerminalRole::DocumentTitle));
    assert!(roles.contains(&TerminalRole::Heading { level: 1 }));
    assert!(roles.contains(&TerminalRole::Rule));
}

#[test]
fn a_link_keeps_its_target_for_a_host_that_can_use_it() {
    let rendered = document("See https://example.com[the site].\n", 60);
    let link = rendered.lines[0]
        .spans
        .iter()
        .find(|span| span.style.role == TerminalRole::Link)
        .expect("link span");

    assert_eq!(link.text, "the site");
    assert_eq!(link.link.as_deref(), Some("https://example.com"));
}

#[test]
fn an_unlimited_width_leaves_every_paragraph_on_one_line() {
    let rendered = render(
        analyze("one two three four five six seven eight nine ten\n").document(),
        &TerminalPolicy {
            width: TerminalWidth::Unlimited,
            ..TerminalPolicy::default()
        },
    );

    assert_eq!(
        rendered.document.to_plain_text(),
        "one two three four five six seven eight nine ten\n"
    );
}

/// Deep indentation is allowed to push a block past the requested width. Text
/// squeezed into two or three columns cannot be read at all, while a block that
/// overflows by a few columns still can.
#[test]
fn indentation_never_squeezes_the_text_below_the_minimum_content_width() {
    let policy = policy(30);
    let mut canvas = Canvas::new(&policy);

    assert_eq!(canvas.content_width(), Some(30));
    canvas.indented(24, |canvas| {
        assert_eq!(canvas.content_width(), Some(20));
    });
}

/// A width narrower than that minimum is still honored, because the reader
/// asked for it and nothing is nested inside.
#[test]
fn a_narrow_width_without_indentation_is_honored() {
    let policy = policy(10);
    let canvas = Canvas::new(&policy);

    assert_eq!(canvas.content_width(), Some(10));
}

#[test]
fn the_plain_text_matches_the_text_of_every_span() {
    let rendered = document("= Title\n\n*bold* text\n\n== Section\n", 40);
    let joined: String = rendered
        .lines
        .iter()
        .map(|line| format!("{}\n", line.text()))
        .collect();

    assert_eq!(joined, rendered.to_plain_text());
}

#[test]
fn width_counts_two_columns_for_wide_characters_and_none_for_combining_marks() {
    assert_eq!(display_width("日本語", AmbiguousWidth::Narrow), 6);
    assert_eq!(display_width("ab", AmbiguousWidth::Narrow), 2);
    assert_eq!(display_width("e\u{301}", AmbiguousWidth::Narrow), 1);
}

/// Characters of ambiguous East Asian width follow the host's setting.
#[test]
fn ambiguous_width_follows_the_requested_setting() {
    assert_eq!(display_width("…", AmbiguousWidth::Narrow), 1);
    assert_eq!(display_width("…", AmbiguousWidth::Wide), 2);
}

#[test]
fn a_combining_mark_stays_with_the_character_it_belongs_to() {
    let spans = vec![TerminalSpan::new("ae\u{301}iou", TerminalStyle::default())];
    let lines = wrap_spans(&spans, Some(2), AmbiguousWidth::Narrow);
    let texts: Vec<String> = lines
        .iter()
        .map(|line| line.iter().map(|span| span.text.as_str()).collect())
        .collect();

    assert_eq!(
        texts,
        vec!["ae\u{301}".to_owned(), "io".to_owned(), "u".to_owned()]
    );
}

#[test]
fn control_characters_never_reach_a_rendered_line() {
    let spans = vec![TerminalSpan::new(
        "before\u{1b}[31mafter",
        TerminalStyle::default(),
    )];
    let lines = wrap_spans(&spans, None, AmbiguousWidth::Narrow);
    let text: String = lines[0].iter().map(|span| span.text.as_str()).collect();

    assert_eq!(text, "before[31mafter");
}

#[test]
fn an_indented_line_carries_its_indentation_and_no_line_ends_in_spaces() {
    let policy = TerminalPolicy {
        indent: IndentPolicy {
            heading_step: 2,
            verbatim: 4,
        },
        ..policy(40)
    };
    let mut canvas = Canvas::new(&policy);
    canvas.indented(4, |canvas| {
        canvas.push_text("text", TerminalStyle::default());
        canvas.blank_line();
    });
    let lines = canvas.finish();

    assert_eq!(lines[0].text(), "    text");
    assert_eq!(lines[1].text(), "");
}
