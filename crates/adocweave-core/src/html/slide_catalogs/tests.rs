use super::*;
use crate::generated_bibliography::{GeneratedBibliography, GeneratedBibliographyEntry};
use crate::html::{self, HtmlSlideRegions, RenderPolicy, render_slide_regions};
use crate::{AnalysisOptions, Engine, OutputLimits};

fn region(document: &Document, roots: &[usize]) -> HtmlRegionSelection {
    HtmlRegionSelection {
        blocks: roots
            .iter()
            .map(|index| document.index().top_level_blocks()[*index])
            .collect(),
        ..Default::default()
    }
}

fn render(
    document: &Document,
    inputs: &RenderInputs,
    selections: &HtmlSlideSelections,
    scope: HtmlSlideScope,
) -> HtmlSlideRegions {
    render_slide_regions(
        document,
        &RenderPolicy::default(),
        inputs,
        selections,
        scope,
        &BTreeSet::new(),
        OutputLimits::default(),
    )
    .unwrap()
}

fn bibliography(scope: HtmlSlideScope, keys: &[&str]) -> RenderInputs {
    RenderInputs::default().with_generated_bibliography(
        GeneratedBibliography::new(
            "References",
            keys.iter()
                .map(|key| {
                    GeneratedBibliographyEntry::new(*key, format!("Work {key}"))
                        .with_label(format!("[{key}]"))
                })
                .collect(),
        )
        .with_namespace(scope.namespace()),
    )
}

#[test]
fn selected_captions_and_automatic_xrefs_are_numbered_per_scope_without_mutation() {
    let analysis = Engine::new(AnalysisOptions::default()).analyze(concat!(
        "[#body-first]\n.First\nimage::first.png[]\n\n",
        "[.notes]\n--\n[#notes-first]\n.Private figure\nimage::private.png[]\n\nSee xref:#notes-first[] and xref:#body-last[].\n--\n\n",
        "[#body-last]\n.Last\nimage::last.png[]\n\n",
        "See xref:#body-last[] and xref:#body-last[chosen label].\n"
    )).unwrap();
    let doc = analysis.document();
    let before_targets = doc.reference_targets().to_vec();
    let before_captions = doc.presentation().captions().to_vec();
    let before_html = html::render(doc, &RenderPolicy::default());
    let before_terminal = crate::terminal::render(doc, &crate::terminal::TerminalPolicy::default());
    let before_projection = crate::projection::block_presentations(&analysis);
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[0])], vec![region(doc, &[2, 3])]],
        notes: vec![vec![region(doc, &[1])], vec![]],
    };
    let body = render(
        doc,
        &RenderInputs::default(),
        &selections,
        HtmlSlideScope::Body,
    );
    let notes = render(
        doc,
        &RenderInputs::default(),
        &selections,
        HtmlSlideScope::Notes,
    );
    let body_html = body.regions.concat();
    assert!(body_html.contains("Figure 1. First"), "{body_html}");
    assert!(body_html.contains("Figure 2. Last"), "{body_html}");
    assert!(
        body_html.contains("href=\"#body-last\">Figure 2</a>"),
        "{body_html}"
    );
    assert!(body_html.contains("href=\"#body-last\">chosen label</a>"));
    assert!(notes.regions.concat().contains("Figure 1. Private figure"));
    assert!(
        notes
            .regions
            .concat()
            .contains("href=\"#notes-first\">Figure 1</a>")
    );
    assert!(
        notes
            .regions
            .concat()
            .contains("href=\"#body-last\">Figure 2</a>")
    );
    assert!(!body_html.contains("private.png"));
    assert!(before_html.html.contains("Figure 3. Last"));
    assert_eq!(doc.reference_targets(), before_targets);
    assert_eq!(doc.presentation().captions(), before_captions);
    assert_eq!(html::render(doc, &RenderPolicy::default()), before_html);
    assert_eq!(
        crate::terminal::render(doc, &crate::terminal::TerminalPolicy::default()),
        before_terminal
    );
    assert_eq!(
        crate::projection::block_presentations(&analysis),
        before_projection
    );
}

#[test]
fn captions_keep_family_prefix_and_disabled_numbering_at_the_source_position() {
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze(concat!(
            ":figure-caption: Fig.\n:listing-caption: Code\n\n",
            ".First\nimage::a.png[]\n\n",
            ".Table\n|===\n|A\n|===\n\n",
            ".Code\n[source,rust]\n----\nlet n = 1;\n----\n\n",
            ":figure-caption!:\n\n.Unnumbered\nimage::b.png[]\n"
        ))
        .unwrap();
    let doc = analysis.document();
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[0, 1, 2, 3])]],
        notes: vec![],
    };
    let output = render(
        doc,
        &RenderInputs::default(),
        &selections,
        HtmlSlideScope::Body,
    )
    .regions
    .concat();
    assert!(output.contains("Fig. 1. First"), "{output}");
    assert!(output.contains("Table 1. Table"));
    assert!(output.contains("Code 1. Code"));
    assert!(output.contains("<figcaption>Unnumbered</figcaption>"));
}

#[test]
fn footnotes_are_once_per_slide_across_columns_and_shared_content_is_copied_to_notes() {
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze(concat!(
            "First footnote:shared[Public *detail*].\n\n",
            "Another footnote:[Other content].\n\n",
            "Again footnote:shared[].\n\n",
            "[.notes]\n--\nNotes footnote:shared[] and footnote:[Private detail].\n--\n"
        ))
        .unwrap();
    let doc = analysis.document();
    let selections = HtmlSlideSelections {
        body: vec![
            vec![region(doc, &[0]), region(doc, &[1])],
            vec![region(doc, &[2])],
        ],
        notes: vec![vec![], vec![region(doc, &[3])]],
    };
    let body = render(
        doc,
        &RenderInputs::default(),
        &selections,
        HtmlSlideScope::Body,
    );
    let notes = render(
        doc,
        &RenderInputs::default(),
        &selections,
        HtmlSlideScope::Notes,
    );
    assert_eq!(body.regions.len(), 3);
    assert_eq!(body.footnotes.len(), 2);
    assert_eq!(
        body.footnotes[0]
            .matches("Public <strong>detail</strong>")
            .count(),
        1
    );
    assert_eq!(
        body.footnotes[1]
            .matches("Public <strong>detail</strong>")
            .count(),
        1
    );
    assert!(body.footnotes[0].contains("id=\"slides-body-s1-footnote-1\" value=\"1\""));
    assert!(body.footnotes[0].contains("id=\"slides-body-s1-footnote-2\" value=\"2\""));
    assert!(body.footnotes[1].contains("id=\"slides-body-s2-footnote-1\" value=\"1\""));
    assert!(body.regions[2].contains("href=\"#slides-body-s2-footnote-1\">1</a>"));
    assert!(!body.footnotes.concat().contains("Private detail"));
    assert!(notes.footnotes[0].is_empty());
    assert!(notes.footnotes[1].contains("id=\"slides-notes-s2-footnote-1\" value=\"1\""));
    assert!(notes.footnotes[1].contains("Public <strong>detail</strong>"));
    assert!(notes.footnotes[1].contains("id=\"slides-notes-s2-footnote-2\" value=\"2\""));
    assert!(notes.footnotes[1].contains("Private detail"));
    // Local backrefs never jump to another slide's placement of the same note.
    assert!(!body.footnotes[0].contains("#slides-body-s2-"));
    assert!(!body.footnotes[1].contains("#slides-body-s1-"));
    let normal = html::render(doc, &RenderPolicy::default()).html;
    assert_eq!(normal.matches("id=\"_footnote_1\"").count(), 1);
    assert!(normal.contains("id=\"_footnote_3\""));
}

#[test]
fn body_cannot_pull_a_definition_from_notes_in_either_audience() {
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze(
            "Public footnote:secret[].\n\n[.notes]\n--\nfootnote:secret[PRIVATE_CONTENT]\n--\n",
        )
        .unwrap();
    let doc = analysis.document();
    let reference = doc.catalogs().footnotes()[0]
        .occurrences
        .iter()
        .find(|occurrence| occurrence.range != doc.catalogs().footnotes()[0].definition_range)
        .unwrap()
        .range;
    for notes in [vec![], vec![vec![region(doc, &[1])]]] {
        let selections = HtmlSlideSelections {
            body: vec![vec![region(doc, &[0])]],
            notes,
        };
        assert_eq!(
            render_slide_regions(
                doc,
                &RenderPolicy::default(),
                &RenderInputs::default(),
                &selections,
                HtmlSlideScope::Body,
                &BTreeSet::new(),
                OutputLimits::default()
            )
            .unwrap_err(),
            HtmlRegionError::FootnoteOutsideScope { range: reference }
        );
    }
}

#[test]
fn generated_bibliographies_keep_same_keys_and_only_selected_scope_backrefs() {
    let analysis = Engine::new(AnalysisOptions::default()).analyze("First cite:[shared].\n\n[.notes]\n--\nPrivate cite:[shared] and cite:[private].\n--\n\nLast cite:[shared].\n").unwrap();
    let doc = analysis.document();
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[0])], vec![region(doc, &[2])]],
        notes: vec![vec![region(doc, &[1])], vec![]],
    };
    let body = render(
        doc,
        &bibliography(HtmlSlideScope::Body, &["shared"]),
        &selections,
        HtmlSlideScope::Body,
    );
    let notes = render(
        doc,
        &bibliography(HtmlSlideScope::Notes, &["shared", "private"]),
        &selections,
        HtmlSlideScope::Notes,
    );
    assert!(body.diagnostics.is_empty(), "{:?}", body.diagnostics);
    assert!(notes.diagnostics.is_empty(), "{:?}", notes.diagnostics);
    let body_bib = body.bibliography.as_ref().unwrap();
    let note_bib = notes.bibliography.as_ref().unwrap();
    assert!(body_bib.contains("id=\"slides-body-bib-shared\""));
    assert!(note_bib.contains("id=\"slides-notes-bib-shared\""));
    assert_eq!(
        body_bib.matches("class=\"bibliography-backref\"").count(),
        2
    );
    assert_eq!(
        note_bib.matches("class=\"bibliography-backref\"").count(),
        2
    );
    assert!(body_bib.contains("#slides-body-bib-ref-"));
    assert!(!body_bib.contains("slides-notes"));
    assert!(note_bib.contains("#slides-notes-bib-ref-"));
    assert!(!note_bib.contains("slides-body"));
    assert!(
        notes
            .bibliography
            .unwrap()
            .contains("id=\"slides-notes-references\"")
    );
    assert!(body.generated_ids.contains("slides-body-references"));
    assert!(
        body.regions
            .concat()
            .contains("href=\"#slides-body-bib-shared\"")
    );
    assert!(!body.regions.concat().contains("private"));
}

#[test]
fn collisions_cover_bibliography_containers_anchors_landings_and_footnote_placements() {
    for (id, content, with_bib) in [
        ("slides-body-references", "cite:[item]", true),
        ("slides-body-bib-item", "cite:[item]", true),
        ("slides-body-s1-footnote-1", "footnote:[Text]", false),
        ("slides-body-s1-footnote-ref-1", "footnote:[Text]", false),
        ("slides-body-bib-ref-1", "cite:[item]", true),
    ] {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(&format!("[#{}]\n{}\n", id, content))
            .unwrap();
        let doc = analysis.document();
        let selections = HtmlSlideSelections {
            body: vec![vec![region(doc, &[0])]],
            notes: vec![],
        };
        let inputs = if with_bib {
            bibliography(HtmlSlideScope::Body, &["item"])
        } else {
            RenderInputs::default()
        };
        assert!(
            matches!(render_slide_regions(doc, &RenderPolicy::default(), &inputs, &selections, HtmlSlideScope::Body, &BTreeSet::new(), OutputLimits::default()), Err(HtmlRegionError::GeneratedIdCollision {id: collided,..}) if collided == id)
        );
    }
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("cite:[item]\n")
        .unwrap();
    let doc = analysis.document();
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[0])]],
        notes: vec![],
    };
    let output = render(
        doc,
        &bibliography(HtmlSlideScope::Body, &["item"]),
        &selections,
        HtmlSlideScope::Body,
    );
    let landing = output
        .generated_ids
        .iter()
        .find(|id| id.contains("-bib-ref-"))
        .unwrap();
    assert!(
        matches!(render_slide_regions(doc,&RenderPolicy::default(),&bibliography(HtmlSlideScope::Body,&["item"]),&selections,HtmlSlideScope::Body,&BTreeSet::from([landing.clone()]),OutputLimits::default()),Err(HtmlRegionError::GeneratedIdCollision {id,..}) if id == *landing)
    );
}

#[test]
fn output_limit_counts_footer_and_bibliography_with_regions() {
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("Text footnote:[Footer] cite:[item].\n")
        .unwrap();
    let doc = analysis.document();
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[0])]],
        notes: vec![],
    };
    let inputs = bibliography(HtmlSlideScope::Body, &["item"]);
    let output = render(doc, &inputs, &selections, HtmlSlideScope::Body);
    let bytes = output
        .regions
        .iter()
        .chain(&output.footnotes)
        .map(String::len)
        .sum::<usize>()
        + output.bibliography.as_ref().unwrap().len();
    assert!(
        render_slide_regions(
            doc,
            &RenderPolicy::default(),
            &inputs,
            &selections,
            HtmlSlideScope::Body,
            &BTreeSet::new(),
            OutputLimits {
                max_output_bytes: bytes as u32
            }
        )
        .is_ok()
    );
    assert!(
        matches!(render_slide_regions(doc,&RenderPolicy::default(),&inputs,&selections,HtmlSlideScope::Body,&BTreeSet::new(),OutputLimits{max_output_bytes:bytes as u32-1}),Err(HtmlRegionError::OutputLimit {actual,..}) if actual == bytes)
    );
}

#[test]
fn nested_omission_excludes_footnotes_and_caption_titles_are_selected_content() {
    let analysis = Engine::new(AnalysisOptions::default()).analyze(concat!(
        "====\nPublic footnote:[Visible].\n\n[.notes]\n--\nPrivate footnote:[SECRET].\n--\n====\n\n",
        ".Caption footnote:[Caption detail]\nimage::a.png[]\n"
    )).unwrap();
    let doc = analysis.document();
    let note = doc.catalogs().footnotes()[1].definition_range;
    let mut note_block = None;
    walker::walk(doc, |node| {
        if let SemanticNode::Block(crate::block_model::AstBlock::Delimited(block)) = node
            && block.kind == crate::block_model::DelimitedBlockKind::Open
        {
            note_block = doc.index().block_id_at(block.range);
        }
    });
    let note_block = note_block.unwrap();
    let mut selected = region(doc, &[0, 1]);
    selected.omitted_blocks.insert(note_block);
    let selections = HtmlSlideSelections {
        body: vec![vec![selected]],
        notes: vec![],
    };
    let result = render(
        doc,
        &RenderInputs::default(),
        &selections,
        HtmlSlideScope::Body,
    );
    assert!(result.footnotes[0].contains("Visible"));
    assert!(result.footnotes[0].contains("Caption detail"));
    assert!(!result.footnotes[0].contains("SECRET"));
    assert!(result.regions[0].contains("Figure 1. Caption"));
    assert!(
        !result
            .generated_ids
            .iter()
            .any(|id| id.ends_with(&format!("ref-{}", note.start().to_u32())))
    );
}

#[test]
fn body_to_notes_local_reference_fails_without_disclosing_its_automatic_label() {
    let analysis = Engine::new(AnalysisOptions::default()).analyze("See xref:#secret[].\n\n[.notes]\n--\n[#secret]\n.PRIVATE_LABEL\nimage::secret.png[]\n--\n").unwrap();
    let doc = analysis.document();
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[0])]],
        notes: vec![vec![region(doc, &[1])]],
    };
    assert!(matches!(
        render_slide_regions(
            doc,
            &RenderPolicy::default(),
            &RenderInputs::default(),
            &selections,
            HtmlSlideScope::Body,
            &BTreeSet::new(),
            OutputLimits::default()
        ),
        Err(HtmlRegionError::ReferenceOutsideScope { .. })
    ));
}

#[test]
fn a_private_target_in_footnote_prose_is_diagnosed_without_its_resolved_label() {
    let analysis = Engine::new(AnalysisOptions::default()).analyze("Public footnote:[<<secret>>].\n\n[.notes]\n--\n[#secret]\n.PRIVATE_LABEL\nSecret text.\n--\n").unwrap();
    let doc = analysis.document();
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[0])]],
        notes: vec![vec![region(doc, &[1])]],
    };
    let output = render(
        doc,
        &RenderInputs::default(),
        &selections,
        HtmlSlideScope::Body,
    );
    assert!(!output.footnotes.concat().contains("PRIVATE_LABEL"));
    assert!(!output.footnotes.concat().contains("href=\"#secret\""));
    let diagnostic = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_str() == "slides-reference-outside-scope")
        .unwrap();
    assert_eq!(diagnostic.severity, crate::diagnostic::Severity::Error);
    assert!(diagnostic.range.start() >= doc.catalogs().footnotes()[0].content_range.start());
    assert!(diagnostic.range.end() <= doc.catalogs().footnotes()[0].content_range.end());
}

#[test]
fn hidden_host_heading_is_a_local_target_only_when_explicitly_declared() {
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("[#hidden]\n[%notitle]\n== Hidden heading\n\nSee xref:#hidden[].\n")
        .unwrap();
    let doc = analysis.document();
    let mut selection = region(doc, &[1]);
    selection
        .container_headings
        .insert(doc.index().top_level_blocks()[0]);
    let selections = HtmlSlideSelections {
        body: vec![vec![selection.clone()]],
        notes: vec![],
    };
    let output = render(
        doc,
        &RenderInputs::default(),
        &selections,
        HtmlSlideScope::Body,
    );
    assert!(output.regions[0].contains("href=\"#hidden\">Hidden heading</a>"));
    assert!(!output.regions[0].contains("<h2"));
    assert!(matches!(
        html::render_regions(
            doc,
            &RenderPolicy::default(),
            &RenderInputs::default(),
            &[selection],
            OutputLimits::default()
        ),
        Err(HtmlRegionError::InvalidSelection { .. })
    ));
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[1])]],
        notes: vec![],
    };
    assert!(matches!(
        render_slide_regions(
            doc,
            &RenderPolicy::default(),
            &RenderInputs::default(),
            &selections,
            HtmlSlideScope::Body,
            &BTreeSet::from(["hidden".into()]),
            OutputLimits::default()
        ),
        Err(HtmlRegionError::ReferenceOutsideScope { .. })
    ));
}

#[test]
fn reserved_private_section_or_document_title_is_not_a_host_container_declaration() {
    for heading in ["== PRIVATE_SECTION", "= PRIVATE_DOCUMENT_TITLE"] {
        let analysis = Engine::new(AnalysisOptions::default()).analyze(&format!("[#public]\n[%notitle]\n== Public heading\n\nSee xref:#secret[].\n\n[.notes]\n--\n[#secret]\n{heading}\n--\n")).unwrap();
        let doc = analysis.document();
        let reserved = doc
            .reference_targets()
            .iter()
            .map(|target| target.id.clone())
            .collect();
        let mut body = region(doc, &[1]);
        body.container_headings
            .insert(doc.index().top_level_blocks()[0]);
        let selections = HtmlSlideSelections {
            body: vec![vec![body]],
            notes: vec![vec![region(doc, &[2])]],
        };
        assert!(
            matches!(
                render_slide_regions(
                    doc,
                    &RenderPolicy::default(),
                    &RenderInputs::default(),
                    &selections,
                    HtmlSlideScope::Body,
                    &reserved,
                    OutputLimits::default()
                ),
                Err(HtmlRegionError::ReferenceOutsideScope { .. })
            ),
            "{heading}"
        );
    }
}

#[test]
fn rich_citations_keep_selected_backref_landings_and_scope_ids() {
    use crate::rendered_content::{ResolvedRichCitation, RichInline, ValidatedRichText};
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("cite:[item]\n")
        .unwrap();
    let doc = analysis.document();
    let citation_range = analysis.citations()[0].range;
    let inputs = bibliography(HtmlSlideScope::Body, &["item"]).with_rich_citations(vec![
        ResolvedRichCitation::new(
            citation_range,
            ValidatedRichText::validate(vec![RichInline::Strong {
                children: vec![RichInline::Text { text: "[1]".into() }],
            }])
            .unwrap(),
        ),
    ]);
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[0])]],
        notes: vec![],
    };
    let result = render(doc, &inputs, &selections, HtmlSlideScope::Body);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let landing = result
        .generated_ids
        .iter()
        .find(|id| id.contains("-bib-ref-"))
        .unwrap();
    assert!(result.regions[0].contains(&format!(
        "<span id=\"{landing}\"></span><strong>[1]</strong>"
    )));
    assert!(
        result
            .bibliography
            .unwrap()
            .contains(&format!("href=\"#{landing}\""))
    );
}

#[test]
fn rich_citation_links_have_fixed_accessible_targets_without_nested_anchors() {
    use crate::rendered_content::{ResolvedRichCitation, RichInline, ValidatedRichText};
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("cite:[one, two]\n\n[.notes]\n--\ncite:[one]\n--\n")
        .unwrap();
    let doc = analysis.document();
    let selections = HtmlSlideSelections {
        body: vec![vec![region(doc, &[0])]],
        notes: vec![vec![region(doc, &[1])]],
    };
    let citations = analysis.citations();
    for (scope, citation, keys, target, label) in [
        (
            HtmlSlideScope::Body,
            &citations[0],
            vec!["one", "two"],
            "slides-body-references",
            "Open references",
        ),
        (
            HtmlSlideScope::Notes,
            &citations[1],
            vec!["one"],
            "slides-notes-bib-one",
            "Open cited reference",
        ),
    ] {
        let rich = ValidatedRichText::validate(vec![RichInline::Link {
            href: "https://example.org/work".into(),
            children: vec![RichInline::Text {
                text: "Details".into(),
            }],
        }])
        .unwrap();
        let inputs = bibliography(scope, &keys)
            .with_rich_citations(vec![ResolvedRichCitation::new(citation.range, rich)]);
        let output = render(doc, &inputs, &selections, scope);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let html = output.regions.concat();
        assert!(html.contains(&format!("class=\"citation-link\" href=\"#{target}\" aria-label=\"{label}\" title=\"{label}\">↗</a>")), "{html}");
        let wrapped = format!("<root>{html}</root>");
        let parsed = roxmltree::Document::parse(&wrapped).unwrap();
        for anchor in parsed.descendants().filter(|node| node.has_tag_name("a")) {
            assert!(
                !anchor
                    .ancestors()
                    .skip(1)
                    .any(|node| node.has_tag_name("a"))
            );
        }
        let landing = output
            .generated_ids
            .iter()
            .find(|id| id.contains("-bib-ref-"))
            .unwrap();
        assert!(html.contains(&format!("id=\"{landing}\"")));
        assert!(
            output
                .bibliography
                .as_ref()
                .unwrap()
                .contains(&format!("href=\"#{landing}\""))
        );
        let total = html.len()
            + output.footnotes.iter().map(String::len).sum::<usize>()
            + output.bibliography.unwrap().len();
        assert!(
            render_slide_regions(
                doc,
                &RenderPolicy::default(),
                &inputs,
                &selections,
                scope,
                &BTreeSet::new(),
                OutputLimits {
                    max_output_bytes: total as u32
                }
            )
            .is_ok()
        );
        assert!(matches!(
            render_slide_regions(
                doc,
                &RenderPolicy::default(),
                &inputs,
                &selections,
                scope,
                &BTreeSet::new(),
                OutputLimits {
                    max_output_bytes: total as u32 - 1
                }
            ),
            Err(HtmlRegionError::OutputLimit { .. })
        ));
        let ordinary = html::render_with_inputs(doc, &RenderPolicy::default(), &inputs);
        assert!(!ordinary.html.contains("citation-link"));
    }
}

#[test]
fn repeated_footnote_math_uses_local_copy_targets_and_prose_uses_the_first_placement() {
    use crate::rendered_content::{ResolvedMath, ValidatedMath};
    let analysis = Engine::new(AnalysisOptions::default()).analyze("First footnote:shared[latexmath:[x] latexmath:[y]].\n\nAgain footnote:shared[].\n\nExternal latexmath:[z].\n").unwrap();
    let document = analysis.document();
    let definition = document.catalogs().footnotes()[0].definition_range;
    let mut ranges = Vec::new();
    walker::walk_inlines(document.footnote_body(definition).unwrap(), |node| {
        if let SemanticNode::Inline(Inline::Formula(formula)) = node {
            ranges.push(formula.range);
        }
    });
    ranges.push(crate::projection::formulas(&analysis)[0].source_range);
    let math = ranges
        .iter()
        .enumerate()
        .map(|(index, range)| {
            let key = format!("m{index}");
            let link = if index == 1 {
                String::new()
            } else {
                "<a href=\"#body-m1-i0\"><text>reference</text></a>".into()
            };
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\"><g id=\"body-{key}-i0\">{link}</g></svg>"
            );
            ResolvedMath::new(*range, ValidatedMath::validate("body", &key, &svg).unwrap())
        })
        .collect();
    let inputs = RenderInputs::default().with_math(math);
    let selections = HtmlSlideSelections {
        body: vec![
            vec![region(document, &[0])],
            vec![region(document, &[1])],
            vec![region(document, &[2])],
        ],
        notes: vec![],
    };
    let ordinary = html::render(document, &RenderPolicy::default());
    let terminal = crate::terminal::render(document, &crate::terminal::TerminalPolicy::default());
    let catalogs = document.catalogs().footnotes().to_vec();
    let output = render(document, &inputs, &selections, HtmlSlideScope::Body);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(output.footnotes[0].contains("href=\"#slides-body-s1-footnote-1-body-m1-i0\""));
    assert!(output.footnotes[1].contains("href=\"#slides-body-s2-footnote-1-body-m1-i0\""));
    assert!(output.regions[2].contains("href=\"#slides-body-s1-footnote-1-body-m1-i0\""));
    assert_eq!(
        output
            .generated_ids
            .iter()
            .filter(|id| id.ends_with("-body-m1-i0"))
            .count(),
        2
    );
    assert!(!output.generated_ids.contains("body-m1-i0"));
    assert_eq!(html::render(document, &RenderPolicy::default()), ordinary);
    assert_eq!(
        crate::terminal::render(document, &crate::terminal::TerminalPolicy::default()),
        terminal
    );
    assert_eq!(document.catalogs().footnotes(), catalogs);
    let collision = render_slide_regions(
        document,
        &RenderPolicy::default(),
        &inputs,
        &selections,
        HtmlSlideScope::Body,
        &BTreeSet::from(["slides-body-s2-footnote-1-body-m1-i0".into()]),
        OutputLimits::default(),
    );
    assert!(
        matches!(collision, Err(HtmlRegionError::GeneratedIdCollision {range, ..}) if range == ranges[1])
    );
    let bounded = render_slide_regions(
        document,
        &RenderPolicy::default(),
        &inputs,
        &selections,
        HtmlSlideScope::Body,
        &BTreeSet::new(),
        OutputLimits {
            max_output_bytes: 300,
        },
    );
    assert!(matches!(bounded, Err(HtmlRegionError::OutputLimit { .. })));
}

#[test]
fn manual_bibliography_keeps_local_xref_backrefs_and_all_footnote_citation_placements() {
    let analysis = Engine::new(AnalysisOptions::default()).analyze("First footnote:shared[cite:[manual]].\n\nAgain footnote:shared[].\n\nSee xref:#manual[].\n\n[bibliography]\n== References\n\n* [[[manual]]] Entry.\n").unwrap();
    let document = analysis.document();
    let selections = HtmlSlideSelections {
        body: vec![
            vec![region(document, &[0])],
            vec![region(document, &[1])],
            vec![region(document, &[2, 3, 4])],
        ],
        notes: vec![],
    };
    let output = render(
        document,
        &RenderInputs::default(),
        &selections,
        HtmlSlideScope::Body,
    );
    let bibliography = &output.regions[2];
    assert_eq!(
        bibliography
            .matches("class=\"bibliography-backref\"")
            .count(),
        3,
        "{bibliography}"
    );
    assert!(bibliography.contains("href=\"#slides-body-s1-footnote-1-bib-ref-"));
    assert!(bibliography.contains("href=\"#slides-body-s2-footnote-1-bib-ref-"));
    assert!(bibliography.contains("href=\"#slides-body-bib-ref-"));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn slide_footnotes_reject_authored_anchor_definitions_without_repeating_their_ids() {
    for anchor in [
        "anchor:inside[]",
        "[[inside]]",
        "bibanchor:inside[]",
        "[[[inside]]]",
    ] {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(&format!(
                "First footnote:shared[{anchor} detail].\n\nAgain footnote:shared[].\n"
            ))
            .unwrap();
        let document = analysis.document();
        let selections = HtmlSlideSelections {
            body: vec![vec![region(document, &[0])], vec![region(document, &[1])]],
            notes: vec![],
        };
        let output = render(
            document,
            &RenderInputs::default(),
            &selections,
            HtmlSlideScope::Body,
        );
        assert!(
            !output.footnotes.concat().contains("id=\"inside\""),
            "{anchor}"
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == "slides-footnote-anchor-unsupported"),
            "{anchor}"
        );
        assert!(
            html::render(document, &RenderPolicy::default())
                .html
                .contains("id=\"inside\""),
            "{anchor}"
        );
    }
}
