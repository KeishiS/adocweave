use adocweave_core::output::{html, projection, terminal};
use adocweave_core::semantic::{Block, DelimitedBlockKind, SemanticNode, walk};
use adocweave_core::{AnalysisOptions, Engine};

#[test]
fn comment_titles_preserve_source_without_contributing_semantic_content() {
    for title in [
        "footnote:[Private note]",
        "xref:#private[]",
        "cite:[private]",
        "latexmath:[x=1]",
        "image:private.png[]",
        "https://example.com/private[Private link]",
        "{counter:private}",
        "{counter2:private}",
    ] {
        let source = format!(".{title}\n////\nPrivate comment.\n////\n\nVisible.\n");
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(&source)
            .expect("analysis succeeds");

        assert_eq!(analysis.source(), source);
        assert_eq!(analysis.syntax().reconstruct(), source);
        let Block::Delimited(comment) = &analysis.document().blocks()[0] else {
            panic!("comment block");
        };
        assert_eq!(comment.kind, DelimitedBlockKind::Comment);
        assert!(comment.metadata.title.is_none());
        assert!(analysis.diagnostics().is_empty(), "{title}");
        assert!(analysis.catalogs().footnotes().is_empty(), "{title}");
        assert!(analysis.references().is_empty(), "{title}");
        assert!(analysis.reference_queries().is_empty(), "{title}");
        assert!(analysis.macros().is_empty(), "{title}");
        assert!(analysis.citations().is_empty(), "{title}");
        assert!(analysis.links().is_empty(), "{title}");
        assert!(analysis.resource_queries().is_empty(), "{title}");
        assert!(analysis.attribute_references().is_empty(), "{title}");
        assert!(projection::formulas(&analysis).is_empty(), "{title}");
        assert!(
            !analysis
                .attribute_environment()
                .final_values()
                .contains_key("private"),
            "{title}"
        );
        let html = html::render(analysis.document(), &html::RenderPolicy::default());
        assert_eq!(html.html, "<p>Visible.</p>\n", "{title}");
        assert!(html.diagnostics.is_empty(), "{title}");
        let terminal = terminal::render(analysis.document(), &terminal::TerminalPolicy::default());
        assert_eq!(terminal.document.to_plain_text(), "Visible.\n", "{title}");
        assert!(terminal.diagnostics.is_empty(), "{title}");
    }
}

#[test]
fn nested_comment_titles_do_not_advance_counters_or_footnote_numbers() {
    let source = concat!(
        ".{counter:n:10} footnote:[First private note]\n",
        "////\nPrivate.\n////\n\n",
        "--\n",
        ".{counter2:n:20} footnote:[Second private note]\n",
        "////\nPrivate.\n////\n",
        "--\n\n",
        "* item\n+\n",
        ".{counter:n:30} footnote:[Third private note]\n",
        "////\nPrivate.\n////\n\n",
        "[cols=\"a\"]\n|===\n|\n",
        ".{counter:n:40} footnote:[Fourth private note]\n",
        "////\nPrivate.\n////\n",
        "|===\n\n",
        ".Visible {counter:n} footnote:[Public note]\nBody.\n",
    );
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze(source)
        .expect("analysis succeeds");
    assert_eq!(analysis.syntax().reconstruct(), source);
    let mut comments = 0;
    walk(analysis.document(), |node| {
        if let SemanticNode::Block(Block::Delimited(block)) = node
            && block.kind == DelimitedBlockKind::Comment
        {
            comments += 1;
            assert!(block.metadata.title.is_none());
        }
    });
    assert_eq!(comments, 4);
    assert_eq!(analysis.catalogs().footnotes().len(), 1);
    assert_eq!(analysis.catalogs().footnotes()[0].number, 1);
    assert_eq!(analysis.catalogs().footnotes()[0].text, "Public note");
    assert_eq!(
        analysis
            .attribute_environment()
            .final_values()
            .get("n")
            .map(String::as_str),
        Some("1")
    );
    let html = html::render(analysis.document(), &html::RenderPolicy::default());
    assert!(html.html.contains("Visible 1 <sup"), "{}", html.html);
    assert!(!html.html.contains("private"));
    let terminal = terminal::render(analysis.document(), &terminal::TerminalPolicy::default());
    assert!(terminal.document.to_plain_text().contains("Visible 1 [1]"));
    assert!(!terminal.document.to_plain_text().contains("private"));
}
