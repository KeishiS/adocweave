use adocweave::output::projection::{SearchTextKind, searchable_text};
use adocweave::{AnalysisOptions, Engine};
use adocweave_textlint::{TxtAstNode, Utf16Range};

#[test]
fn textlint_and_search_projection_separate_representative_prose_and_code() {
    let source = concat!(
        "= 見出し\n\n",
        "本文 `inline` link:https://example.com[リンク]。\n\n",
        "[source,rust]\n----\nfn main() {}\n----\n\n",
        "|===\n|表の本文\n|===\n",
    );
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze(source)
        .expect("representative analysis");
    let plan = adocweave_textlint::plan(&analysis, adocweave_textlint::PlanLimits::default())
        .expect("representative textlint plan");
    assert_eq!(
        plan.range,
        Utf16Range(
            0,
            u32::try_from(source.encode_utf16().count()).expect("UTF-16 length")
        )
    );

    let mut code = Vec::new();
    let mut prose = Vec::new();
    collect_ranges(&plan.children, &mut code, &mut prose);
    assert!(!code.is_empty(), "representative input must contain code");
    assert!(!prose.is_empty(), "representative input must contain prose");

    let search_segments = searchable_text(&analysis).segments;
    assert!(
        search_segments
            .iter()
            .any(|segment| segment.kind == SearchTextKind::Prose),
        "検索投影には本文が必要です"
    );
    assert!(
        search_segments
            .iter()
            .any(|segment| segment.kind == SearchTextKind::Code),
        "検索投影にはコードが必要です"
    );

    for segment in search_segments {
        let segment_range = (
            utf16_offset(source, segment.source_range.start().to_usize()),
            utf16_offset(source, segment.source_range.end().to_usize()),
        );
        let opposite = match segment.kind {
            SearchTextKind::Prose => &code,
            SearchTextKind::Code => &prose,
        };
        assert!(
            opposite
                .iter()
                .all(|range| !overlaps(*range, segment_range)),
            "textlintと検索投影で本文とコードの分類が一致していません"
        );
    }
}

fn collect_ranges(nodes: &[TxtAstNode], code: &mut Vec<Utf16Range>, prose: &mut Vec<Utf16Range>) {
    for node in nodes {
        match node {
            TxtAstNode::CodeBlock { range, .. } => code.push(*range),
            TxtAstNode::Str { range, .. } => prose.push(*range),
            TxtAstNode::Header { children, .. }
            | TxtAstNode::Paragraph { children, .. }
            | TxtAstNode::List { children, .. }
            | TxtAstNode::ListItem { children, .. }
            | TxtAstNode::BlockQuote { children, .. }
            | TxtAstNode::Table { children, .. }
            | TxtAstNode::TableRow { children, .. }
            | TxtAstNode::TableCell { children, .. }
            | TxtAstNode::Strong { children, .. }
            | TxtAstNode::Emphasis { children, .. }
            | TxtAstNode::Link { children, .. } => collect_ranges(children, code, prose),
            _ => {}
        }
    }
}

fn utf16_offset(source: &str, byte_offset: usize) -> u32 {
    u32::try_from(source[..byte_offset].encode_utf16().count()).expect("UTF-16 offset")
}

fn overlaps(left: Utf16Range, right: (u32, u32)) -> bool {
    left.0 < right.1 && right.0 < left.1
}
