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
        "│  literal line one\n│    literal line two\n"
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
            nested: 2,
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

#[test]
fn an_unordered_list_marks_each_level_with_its_own_symbol() {
    assert_eq!(
        plain("* first\n* second\n** nested\n*** deeper\n", 40),
        "• first\n• second\n  ◦ nested\n    ▪ deeper\n"
    );
}

/// The text of an item lines up under itself, so a wrapped item still reads as
/// one item.
#[test]
fn a_wrapped_item_lines_up_under_its_own_text() {
    assert_eq!(
        plain("* one two three four five six seven\n", 20),
        "• one two three four\n  five six seven\n"
    );
}

#[test]
fn an_ordered_list_writes_the_number_the_document_asked_for() {
    assert_eq!(
        plain("[lowerroman]\n. one\n. two\n. three\n", 40),
        "  i. one\n ii. two\niii. three\n"
    );
    assert_eq!(
        plain("[upperalpha]\n. one\n. two\n", 40),
        "A. one\nB. two\n"
    );
    assert_eq!(
        plain("[start=4]\n. four\n. five\n", 40),
        "4. four\n5. five\n"
    );
}

/// The numbers are right-aligned so that the text of every item starts in the
/// same column.
#[test]
fn ordered_numbers_are_aligned_so_the_text_starts_in_one_column() {
    let rendered = plain("[start=9]\n. nine\n. ten\n. eleven\n", 40);

    assert_eq!(rendered, " 9. nine\n10. ten\n11. eleven\n");
}

#[test]
fn an_item_numbered_by_the_author_continues_from_that_number() {
    assert_eq!(
        plain(". one\n5. five\n. six\n", 40),
        "1. one\n5. five\n6. six\n"
    );
}

#[test]
fn a_checklist_item_shows_a_box_the_terminal_can_draw() {
    assert_eq!(
        plain("* [ ] open\n* [x] done\n", 40),
        "• [ ] open\n• [x] done\n"
    );
}

#[test]
fn a_description_list_puts_the_term_above_what_it_means() {
    assert_eq!(
        plain("term:: what it means\nother:: something else\n", 40),
        "term\n  what it means\nother\n  something else\n"
    );
}

#[test]
fn a_callout_list_keeps_the_numbers_of_the_code_it_explains() {
    let source = "----\nlet value = 1; <1>\n----\n<1> what the line does\n";

    assert!(plain(source, 40).contains("<1> what the line does"));
}

/// A continuation and a nested list stay inside the item they belong to.
#[test]
fn an_item_keeps_its_continuation_and_its_nested_list() {
    assert_eq!(
        plain("* item\n** nested\n+\ncontinued text\n", 40),
        "• item\n  ◦ nested\n\n    continued text\n"
    );
}

#[test]
fn a_one_paragraph_admonition_reads_on_the_line_of_its_label() {
    assert_eq!(
        plain("NOTE: remember this and that\n", 24),
        "NOTE: remember this and\n      that\n"
    );
}

/// An admonition of several blocks names its kind once and runs a border down
/// everything it covers.
#[test]
fn an_admonition_block_is_named_once_and_bordered() {
    assert_eq!(
        plain("[WARNING]\n====\nFirst.\n\nSecond.\n====\n", 40),
        "WARNING\n│ First.\n│\n│ Second.\n"
    );
}

#[test]
fn an_admonition_span_carries_the_kind_it_warns_about() {
    let rendered = document("TIP: try this\n", 40);
    let marker = &rendered.lines[0].spans[0];

    assert_eq!(marker.text, "TIP: ");
    assert_eq!(
        marker.style.role,
        TerminalRole::Admonition(crate::block_model::AdmonitionKind::Tip)
    );
}

#[test]
fn a_quotation_is_bordered_and_names_its_source() {
    assert_eq!(
        plain("[quote, Someone, A Book]\n____\nQuoted text.\n____\n", 40),
        "│ Quoted text.\n│\n│ — Someone, A Book\n"
    );
}

/// A verse keeps the line breaks the author wrote, because they are the form.
#[test]
fn a_verse_keeps_the_lines_the_author_wrote() {
    assert_eq!(
        plain("[verse, Poet]\n____\nLine one\nLine two\n____\n", 40),
        "│ Line one\n│ Line two\n│\n│ — Poet\n"
    );
}

#[test]
fn a_listing_block_is_bordered_and_never_reflowed() {
    assert_eq!(
        plain("----\none two three four five six seven\n----\n", 20),
        "│ one two three four five six seven\n"
    );
}

#[test]
fn a_source_block_numbers_its_lines_when_the_document_asks() {
    assert_eq!(
        plain("[source,rust,linenums]\n----\nfirst\nsecond\n----\n", 40),
        "1 │ first\n2 │ second\n"
    );
}

#[test]
fn a_block_title_stands_directly_above_the_block() {
    assert_eq!(plain(".Title\n----\ncode\n----\n", 40), "Title\n│ code\n");
}

#[test]
fn an_example_and_a_sidebar_are_framed_so_the_container_is_visible() {
    assert_eq!(
        plain(".Example title\n====\nBody.\n====\n", 20),
        "Example 1. Example\ntitle\n────────────────────\nBody.\n────────────────────\n"
    );
    assert_eq!(
        plain("****\nAside.\n****\n", 20),
        "────────────────────\nAside.\n────────────────────\n"
    );
}

/// An open block adds nothing of its own, so its content reads as it would
/// outside the block.
#[test]
fn an_open_block_adds_nothing_of_its_own() {
    assert_eq!(plain("--\nBody.\n--\n", 40), "Body.\n");
}

/// A terminal page cannot be unfolded, so a collapsible block is always open.
#[test]
fn a_collapsible_block_is_open_and_marked_as_one() {
    assert_eq!(
        plain(".More\n[%collapsible]\n====\nHidden.\n====\n", 40),
        "▾ More\n  Hidden.\n"
    );
}

#[test]
fn a_comment_block_is_written_for_the_author_and_never_shown() {
    assert_eq!(
        plain("////\nnot for the reader\n////\n\ntext\n", 40),
        "text\n"
    );
}

#[test]
fn a_bordered_block_never_ends_a_line_in_spaces() {
    let rendered = document("[NOTE]\n====\nFirst.\n\nSecond.\n====\n", 40);

    for line in &rendered.lines {
        assert!(
            !line.text().ends_with(' '),
            "line ends in spaces: {:?}",
            line.text()
        );
    }
}

#[test]
fn ordered_numbers_carry_on_past_the_end_of_the_alphabet() {
    use super::numbering::label;
    use crate::block_model::OrderedListStyle;

    assert_eq!(label(1, OrderedListStyle::LowerAlpha), "a");
    assert_eq!(label(26, OrderedListStyle::LowerAlpha), "z");
    assert_eq!(label(27, OrderedListStyle::LowerAlpha), "aa");
    assert_eq!(label(1, OrderedListStyle::LowerGreek), "α");
    assert_eq!(label(24, OrderedListStyle::LowerGreek), "ω");
    assert_eq!(label(25, OrderedListStyle::LowerGreek), "αα");
    assert_eq!(label(4, OrderedListStyle::UpperRoman), "IV");
    assert_eq!(label(1994, OrderedListStyle::LowerRoman), "mcmxciv");
}

/// A number no Roman numeral can write keeps its digits rather than turning
/// into something unreadable.
#[test]
fn a_number_outside_the_roman_range_keeps_its_digits() {
    use super::numbering::label;
    use crate::block_model::OrderedListStyle;

    assert_eq!(label(0, OrderedListStyle::LowerRoman), "0");
    assert_eq!(label(4000, OrderedListStyle::LowerRoman), "4000");
}

#[test]
fn a_reversed_list_counts_down_to_one() {
    assert_eq!(
        plain("[%reversed]\n. last\n. middle\n. first\n", 40),
        "3. last\n2. middle\n1. first\n"
    );
}

#[test]
fn a_table_is_framed_and_its_header_is_ruled_off() {
    let source = "|===\n|Name |Count\n\n|alpha |1\n|beta |22\n|===\n";

    assert_eq!(
        plain(source, 40),
        "\
┌───────┬───────┐
│ Name  │ Count │
├───────┼───────┤
│ alpha │ 1     │
│ beta  │ 22    │
└───────┴───────┘
"
    );
}

/// Every line of a table is exactly as wide as every other, whatever the cells
/// hold.
#[test]
fn every_line_of_a_table_has_the_same_width() {
    let source =
        "|===\n|Name |Description\n\n|alpha |text that has to wrap inside its column\n|===\n";
    let rendered = document(source, 40);
    let widths: Vec<usize> = rendered
        .lines
        .iter()
        .filter(|line| !line.is_empty())
        .map(|line| line.display_width(AmbiguousWidth::Narrow))
        .collect();

    assert!(widths.iter().all(|width| *width == widths[0]), "{widths:?}");
    assert!(widths[0] <= 40);
}

#[test]
fn a_cell_reaching_across_columns_is_not_crossed_by_a_junction() {
    let source = "[cols=\"1,1\"]\n|===\n2+|across both\n|left |right\n|===\n";

    assert_eq!(
        plain(source, 30),
        "\
┌──────────────┐
│ across both  │
│ left │ right │
└──────┴───────┘
"
    );
}

/// A terminal cannot join cells down the page, so a cell that reaches across
/// rows keeps its text on the first of them and leaves the rest empty.
#[test]
fn a_cell_reaching_across_rows_keeps_its_text_on_the_first_row() {
    let source = "[cols=\"1,1\"]\n|===\n.2+|two rows |first\n|second\n|===\n";

    assert_eq!(
        plain(source, 30),
        "\
┌──────────┬────────┐
│ two rows │ first  │
│          │ second │
└──────────┴────────┘
"
    );
}

#[test]
fn a_cell_is_placed_the_way_the_document_asked() {
    let source = "[cols=\"<,^,>\"]\n|===\n|left |center |right\n|a |b |c\n|===\n";

    assert_eq!(
        plain(source, 30),
        "\
┌──────┬────────┬───────┐
│ left │ center │ right │
│ a    │   b    │     c │
└──────┴────────┴───────┘
"
    );
}

#[test]
fn a_table_the_host_cannot_draw_with_boxes_uses_ascii() {
    let source = "|===\n|a |b\n|===\n";
    let rendered = render(
        analyze(source).document(),
        &TerminalPolicy {
            table_borders: super::TableBorders::Ascii,
            ..policy(30)
        },
    );

    assert_eq!(
        rendered.document.to_plain_text(),
        "\
+---+---+
| a | b |
+---+---+
"
    );
}

#[test]
fn a_table_without_borders_separates_its_columns_by_spacing() {
    let source = "|===\n|alpha |beta\n|===\n";
    let rendered = render(
        analyze(source).document(),
        &TerminalPolicy {
            table_borders: super::TableBorders::None,
            ..policy(30)
        },
    );

    assert_eq!(rendered.document.to_plain_text(), "alpha  beta\n");
}

/// A cell written as AsciiDoc holds blocks of its own, which are laid out in
/// the width of the column they sit in.
#[test]
fn a_cell_written_as_asciidoc_lays_its_blocks_out_inside_the_column() {
    let source = "[cols=\"1a,1\"]\n|===\n|* one\n* two\n|plain\n|===\n";
    let rendered = plain(source, 30);

    assert!(rendered.contains("• one"), "{rendered}");
    assert!(rendered.contains("• two"), "{rendered}");
}

#[test]
fn a_table_narrower_than_its_text_keeps_its_columns_readable() {
    let source = "|===\n|first column |second column |third column\n|===\n";
    let rendered = document(source, 20);

    for line in &rendered.lines {
        assert!(
            !line.text().contains("  │") || line.text().ends_with('│'),
            "a column collapsed: {:?}",
            line.text()
        );
    }
    assert!(rendered.lines.len() > 3);
}

#[test]
fn a_table_title_stands_above_the_frame() {
    let source = ".Measurements\n|===\n|a |b\n|===\n";

    assert!(
        plain(source, 30).starts_with("Table 1. Measurements\n┌"),
        "{}",
        plain(source, 30)
    );
}

#[test]
fn a_table_cell_carries_the_role_of_a_heading_in_the_header_row() {
    let source = "|===\n|Name |Count\n\n|alpha |1\n|===\n";
    let rendered = document(source, 30);
    let header = rendered
        .lines
        .iter()
        .flat_map(|line| &line.spans)
        .find(|span| span.text == "Name")
        .expect("a header cell");

    assert_eq!(header.style.role, TerminalRole::TableHeader);
    assert!(header.style.bold);
}

#[test]
fn a_link_is_followed_by_its_address_when_the_text_is_not_the_address() {
    assert_eq!(
        plain(
            "See https://example.com[the site] and https://plain.example.\n",
            70
        ),
        "See the site (https://example.com) and https://plain.example.\n"
    );
}

#[test]
fn a_link_can_be_shown_without_its_address() {
    let rendered = render(
        analyze("See https://example.com[the site].\n").document(),
        &TerminalPolicy {
            links: super::LinkPresentation::TextOnly,
            ..policy(70)
        },
    );

    assert_eq!(rendered.document.to_plain_text(), "See the site.\n");
}

/// The address a host may make followable is the one the policy allows. The
/// text is written either way, because printing an address is harmless.
#[test]
fn only_an_allowed_address_is_attached_to_the_text() {
    let rendered = document("See link:local/page.html[the page].\n", 70);
    let span = rendered.lines[0]
        .spans
        .iter()
        .find(|span| span.text == "the page")
        .expect("the link text");

    assert!(rendered.lines[0].text().contains("local/page.html"));
    assert_eq!(span.link, None);
}

/// A mail address is written out even though the default policy lets no host
/// turn it into something to click: the reader can still copy it.
#[test]
fn a_mail_address_is_written_out_for_the_reader() {
    let rendered = document("Write to mailto:a@example.com[us].\n", 70);

    assert_eq!(
        rendered.lines[0].text(),
        "Write to us (mailto:a@example.com)."
    );
}

/// The address a host may follow is attached to the text of the link.
#[test]
fn an_allowed_address_is_attached_to_the_text_of_its_link() {
    let rendered = document("See https://example.com[the site].\n", 70);
    let span = rendered.lines[0]
        .spans
        .iter()
        .find(|span| span.text == "the site")
        .expect("the link text");

    assert_eq!(span.link.as_deref(), Some("https://example.com"));
}

/// A terminal shows no pictures. What it can say is that one is there and what
/// it is called.
#[test]
fn a_picture_is_named_where_it_would_be_shown() {
    assert_eq!(
        plain("An image: image:diagram.png[A diagram].\n", 70),
        "An image: [Image: A diagram].\n"
    );
    assert_eq!(plain("video::intro.mp4[]\n", 70), "[Video: intro.mp4]\n");
    assert_eq!(plain("audio::track.ogg[]\n", 70), "[Audio: track.ogg]\n");
}

#[test]
fn a_picture_can_be_left_out_altogether() {
    let rendered = render(
        analyze("Text image:diagram.png[A diagram] more.\n").document(),
        &TerminalPolicy {
            media: super::MediaPresentation::Hidden,
            ..policy(70)
        },
    );

    assert_eq!(rendered.document.to_plain_text(), "Text  more.\n");
}

#[test]
fn a_block_picture_is_captioned_the_way_the_document_numbers_it() {
    assert_eq!(
        plain(".Figure title\nimage::figure.png[A figure]\n", 40),
        "Figure 1. Figure title\n[Image: A figure]\n"
    );
}

#[test]
fn a_note_carries_a_number_that_leads_to_the_text_at_the_end() {
    assert_eq!(
        plain("A claim. footnote:[The note.] More.\n", 40),
        "A claim. [1] More.\n\nFootnotes\n[1] The note.\n"
    );
}

#[test]
fn the_contents_are_listed_where_the_document_puts_them() {
    assert_eq!(
        plain("= Guide\n:toc:\n\n== First\n\n=== Nested\n", 40),
        "\
Guide
═════

Contents
First
  Nested

First
─────

  Nested
"
    );
}

#[test]
fn a_cross_reference_reads_as_the_heading_it_points_at() {
    assert_eq!(
        plain("See <<intro>>.\n\n[[intro]]\n== Introduction\n", 40),
        "See Introduction.\n\nIntroduction\n────────────\n"
    );
}

/// A reference that points nowhere keeps the target the author wrote, so the
/// reader can see what is missing, and the problem is reported.
#[test]
fn a_reference_that_points_nowhere_is_reported() {
    let rendered = render(analyze("See <<missing>>.\n").document(), &policy(40));

    assert_eq!(rendered.document.to_plain_text(), "See missing.\n");
    assert_eq!(
        rendered.diagnostics[0].code.as_str(),
        "unresolved-cross-reference"
    );
}

#[test]
fn the_host_decides_what_an_unresolved_reference_shows() {
    let source = "See <<missing>>.\n";
    let hidden = render(
        analyze(source).document(),
        &TerminalPolicy {
            unresolved_references: super::UnresolvedReferenceText::Hidden,
            ..policy(40)
        },
    );

    assert_eq!(hidden.document.to_plain_text(), "See .\n");
}

/// A reference into another document is resolved by the host, and the text it
/// resolved to is what the page shows.
#[test]
fn a_reference_the_host_resolved_reads_as_the_text_the_host_gave() {
    let analysis = analyze("See xref:other.adoc#x[].\n");
    let range = analysis
        .document()
        .blocks()
        .iter()
        .find_map(|block| match block {
            crate::block_model::AstBlock::Paragraph(paragraph) => {
                paragraph.inlines.iter().find_map(|inline| match inline {
                    crate::inline_model::Inline::Reference(reference) => Some(reference.range),
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("a reference");
    let inputs = crate::render::RenderInputs::default().with_references(vec![
        crate::reference::ResolvedReference {
            source_range: range,
            outcome: crate::reference::ResolutionOutcome::Resolved {
                href: "https://example.com/other#x".to_owned(),
                display_text: Some("The other page".to_owned()),
                notices: Vec::new(),
            },
        },
    ]);

    let rendered = super::render_with_inputs(analysis.document(), &policy(40), &inputs);

    assert_eq!(rendered.document.to_plain_text(), "See The other page.\n");
    assert!(rendered.diagnostics.is_empty());
}

#[test]
fn a_formula_is_shown_as_the_author_wrote_it() {
    assert_eq!(
        plain("Formula stem:[a^2] here.\n", 40),
        "Formula a^2 here.\n"
    );
    assert_eq!(plain("[stem]\n++++\nx = y\n++++\n", 40), "│ x = y\n");
}

#[test]
fn a_formula_can_be_left_out_for_a_page_that_cannot_use_it() {
    let rendered = render(
        analyze("[stem]\n++++\nx = y\n++++\n").document(),
        &TerminalPolicy {
            math: super::MathPresentation::Hidden,
            ..policy(40)
        },
    );

    assert_eq!(rendered.document.to_plain_text(), "");
}

#[test]
fn a_key_a_button_and_a_menu_read_as_the_text_they_name() {
    assert_eq!(
        plain(
            "Press kbd:[Ctrl+C], then btn:[OK], then menu:File[Save As].\n",
            70
        ),
        "Press Ctrl+C, then OK, then File › Save As.\n"
    );
}

#[test]
fn an_anchor_and_an_index_term_are_landing_points_and_show_nothing() {
    assert_eq!(
        plain("Text [[here]] and indexterm:[term] more.\n", 40),
        "Text  and  more.\n"
    );
}

/// The whole layout, as a reader sees it with no styling at all. The expected
/// file is the page itself, so a change to any part of the layout is visible
/// in the difference.
#[test]
fn terminal_contract_golden_lays_the_whole_page_out() {
    let source = include_str!("../../../../fixtures/terminal/contract.adoc");

    assert_eq!(
        plain(source, 80),
        include_str!("../../../../fixtures/terminal/contract.width80.txt")
    );
}

/// A host that can lead the reader to the address has no use for it written
/// beside the text. An address it will not follow is written, because it is
/// otherwise lost.
#[test]
fn an_address_is_written_where_the_host_cannot_lead_the_reader_to_it() {
    let source = "See https://example.com[the site] and link:local.html[a page].\n";
    let rendered = render(
        analyze(source).document(),
        &TerminalPolicy {
            links: super::LinkPresentation::TextWhenFollowable,
            ..policy(70)
        },
    );

    assert_eq!(
        rendered.document.to_plain_text(),
        "See the site and a page (local.html).\n"
    );
}

#[test]
fn the_host_can_ask_for_no_addresses_at_all() {
    let source = "See https://example.com[the site] and link:local.html[a page].\n";
    let rendered = render(
        analyze(source).document(),
        &TerminalPolicy {
            links: super::LinkPresentation::TextOnly,
            ..policy(70)
        },
    );

    assert_eq!(
        rendered.document.to_plain_text(),
        "See the site and a page.\n"
    );
}
