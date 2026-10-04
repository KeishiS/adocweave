//! Selected body regions rendered against one immutable document and input set.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::OutputLimits;
use crate::block_model::{AstBlock, DelimitedBlockKind, HeadingKind, ListKind};
use crate::diagnostic::Diagnostic;
use crate::document::Document;
use crate::presentation::BlockId;
use crate::render::{RenderInputProblemKind, RenderInputs};
use crate::source::TextRange;

use super::{
    InlineRenderContext, RenderPolicy, body, generated_bibliography, render_input_diagnostic,
};

/// A body region selected by the host. Source blocks remain in the document.
///
/// This API renders body content only: no document head, TOC, footnote catalog,
/// or generated bibliography is appended. Selected roots are rendered in the
/// supplied order; omitted blocks are skipped even inside compound content.
/// Selected roots must neither repeat nor contain one another across regions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HtmlRegionSelection {
    pub blocks: Vec<BlockId>,
    pub omitted_blocks: BTreeSet<BlockId>,
    /// First fragment index for a plain list's items or a whole open block.
    /// Only fixed `fragment` and `data-fragment-index` attributes are produced.
    pub stepped_blocks: BTreeMap<BlockId, u32>,
    /// Headings whose ID is emitted on the host's surrounding container.
    pub container_headings: BTreeSet<BlockId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HtmlRegions {
    pub regions: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HtmlRegionError {
    InvalidSelection {
        block: BlockId,
        reason: &'static str,
    },
    OutputLimit {
        limit: u32,
        actual: usize,
    },
}

impl fmt::Display for HtmlRegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSelection { block, reason } => {
                write!(
                    formatter,
                    "invalid HTML region block {}: {reason}",
                    block.get()
                )
            }
            Self::OutputLimit { limit, actual } => {
                write!(
                    formatter,
                    "HTML regions exceed the output limit of {limit} bytes ({actual} bytes)"
                )
            }
        }
    }
}

impl std::error::Error for HtmlRegionError {}

#[derive(Clone, Debug, Default)]
pub(super) struct RegionPresentation {
    pub(super) omitted: BTreeSet<TextRange>,
    pub(super) steps: BTreeMap<TextRange, u32>,
    pub(super) container_headings: BTreeSet<TextRange>,
}

impl RegionPresentation {
    fn prepare(
        document: &Document,
        selection: &HtmlRegionSelection,
    ) -> Result<Self, HtmlRegionError> {
        let range = |id| {
            document
                .index()
                .block_range(id)
                .ok_or(HtmlRegionError::InvalidSelection {
                    block: id,
                    reason: "block does not belong to this document",
                })
        };
        let omitted = selection
            .omitted_blocks
            .iter()
            .map(|id| range(*id))
            .collect::<Result<_, _>>()?;
        let mut steps = BTreeMap::new();
        for (&id, &first) in &selection.stepped_blocks {
            let block = document
                .block(id)
                .ok_or(HtmlRegionError::InvalidSelection {
                    block: id,
                    reason: "block does not belong to this document",
                })?;
            let count = match block {
                AstBlock::List(list)
                    if matches!(list.kind, ListKind::Unordered | ListKind::Ordered) =>
                {
                    list.items.len()
                }
                AstBlock::Delimited(block)
                    if block.kind == DelimitedBlockKind::Open && block.presentation.is_none() =>
                {
                    1
                }
                _ => {
                    return Err(HtmlRegionError::InvalidSelection {
                        block: id,
                        reason: "step presentation requires a plain list or open block",
                    });
                }
            };
            if u32::try_from(count.saturating_sub(1))
                .ok()
                .and_then(|offset| first.checked_add(offset))
                .is_none()
            {
                return Err(HtmlRegionError::InvalidSelection {
                    block: id,
                    reason: "fragment index exceeds the supported range",
                });
            }
            steps.insert(range(id)?, first);
        }
        let mut container_headings = BTreeSet::new();
        for &id in &selection.container_headings {
            if !matches!(document.block(id), Some(AstBlock::Heading(_))) {
                return Err(HtmlRegionError::InvalidSelection {
                    block: id,
                    reason: "container heading is not a heading",
                });
            }
            container_headings.insert(range(id)?);
        }
        Ok(Self {
            omitted,
            steps,
            container_headings,
        })
    }
}

/// Renders selected regions with the normal escaped, URL-checked body writer.
///
/// Resolution usage and diagnostics are shared across all regions, so a result
/// used in a later region is not reported as unused in an earlier one. The byte
/// limit applies to the sum of all regions. Document mode and stylesheets do not
/// change this body-only output.
pub fn render_regions(
    document: &Document,
    policy: &RenderPolicy,
    inputs: &RenderInputs,
    selections: &[HtmlRegionSelection],
    limits: OutputLimits,
) -> Result<HtmlRegions, HtmlRegionError> {
    let presentations = selections
        .iter()
        .map(|selection| RegionPresentation::prepare(document, selection))
        .collect::<Result<Vec<_>, _>>()?;
    let mut roots = selections
        .iter()
        .flat_map(|selection| &selection.blocks)
        .map(|&id| {
            document
                .index()
                .block_range(id)
                .map(|range| (range, id))
                .ok_or(HtmlRegionError::InvalidSelection {
                    block: id,
                    reason: "block does not belong to this document",
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    roots.sort_unstable();
    for pair in roots.windows(2) {
        if pair[1].0.start() < pair[0].0.end() || pair[1].0 == pair[0].0 {
            return Err(HtmlRegionError::InvalidSelection {
                block: pair[1].1,
                reason: "selected roots overlap or repeat a block",
            });
        }
    }
    let mut used = BTreeSet::new();
    for (selection, presentation) in selections.iter().zip(&presentations) {
        let mut included = BTreeSet::new();
        for &id in &selection.blocks {
            let root = document
                .block(id)
                .ok_or(HtmlRegionError::InvalidSelection {
                    block: id,
                    reason: "block does not belong to this document",
                })?;
            let walked = crate::walker::try_walk_block_slice(std::slice::from_ref(root), |node| {
                let crate::walker::SemanticNode::Block(block) = node else {
                    return std::ops::ControlFlow::Continue(());
                };
                if presentation.omitted.iter().any(|range| {
                    range.start() <= block.range().start() && block.range().end() <= range.end()
                }) {
                    return std::ops::ControlFlow::Continue(());
                }
                let id = document
                    .index()
                    .block_id_at(block.range())
                    .expect("document blocks are indexed");
                if !used.insert(id) {
                    return std::ops::ControlFlow::Break(id);
                }
                included.insert(id);
                std::ops::ControlFlow::Continue(())
            });
            if let std::ops::ControlFlow::Break(block) = walked {
                return Err(HtmlRegionError::InvalidSelection {
                    block,
                    reason: "selected roots overlap or repeat a block",
                });
            }
        }
        for &block in selection
            .stepped_blocks
            .keys()
            .chain(&selection.container_headings)
        {
            if !included.contains(&block) {
                return Err(HtmlRegionError::InvalidSelection {
                    block,
                    reason: "presentation target is outside the region or omitted",
                });
            }
        }
    }
    let inner = document.inner();
    let body_plan = body::plan_body_traversal(inner, policy);
    let mut scopes = BTreeMap::new();
    for step in body_plan.steps {
        if let body::BodyTraversalStep::Block { block, scope, .. } = step {
            scopes.insert(block.range(), scope);
        }
    }
    let mut diagnostics = Vec::new();
    let bibliography =
        generated_bibliography::prepare(inputs.generated_bibliography(), inner, &mut diagnostics);
    let mut usage = inputs.track_usage();
    let mut regions = Vec::with_capacity(selections.len());
    let mut total = 0usize;
    {
        let mut context = InlineRenderContext {
            policy,
            input_usage: &mut usage,
            diagnostics: &mut diagnostics,
            catalogs: inner.catalogs(),
            identifiers: inner.identifiers(),
            structure: inner.structure(),
            presentation: inner.presentation(),
            generated_bibliography: bibliography.as_ref(),
            region: None,
        };
        for (selection, presentation) in selections.iter().zip(presentations) {
            context.region = Some(presentation);
            let mut html = String::new();
            for &id in &selection.blocks {
                let block = document.block(id).expect("region roots were validated");
                let scope = scopes
                    .get(&block.range())
                    .copied()
                    .or_else(|| {
                        scopes
                            .iter()
                            .find(|(range, _)| {
                                range.start() <= block.range().start()
                                    && block.range().end() <= range.end()
                            })
                            .map(|(_, scope)| *scope)
                    })
                    .unwrap_or_default();
                super::render_block(&mut html, block, policy, &mut context, scope);
                if matches!(block, AstBlock::Heading(heading) if heading.kind == HeadingKind::DocumentTitle)
                    && policy.render_document_title
                    && !context
                        .region
                        .as_ref()
                        .expect("region exists")
                        .omitted
                        .contains(&block.range())
                {
                    super::render_header_metadata(&mut html, inner.header());
                }
                let actual = total.saturating_add(html.len());
                if actual > limits.max_output_bytes as usize {
                    return Err(HtmlRegionError::OutputLimit {
                        limit: limits.max_output_bytes,
                        actual,
                    });
                }
            }
            total += html.len();
            regions.push(html);
        }
    }
    for problem in usage.finish() {
        let domain = problem.domain.as_str();
        let (code, message) = match problem.kind {
            RenderInputProblemKind::Duplicate => (
                "duplicate-render-input",
                format!("multiple {domain} resolutions have the same source range"),
            ),
            RenderInputProblemKind::Unused => (
                "unused-render-input",
                format!("{domain} resolution does not match a renderable {domain}"),
            ),
        };
        diagnostics.push(render_input_diagnostic(
            code,
            domain,
            &message,
            problem.range,
        ));
    }
    crate::diagnostic::sort_diagnostics(&mut diagnostics);
    Ok(HtmlRegions {
        regions,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnalysisOptions, Engine};

    #[test]
    fn selection_rejects_repetition_overlap_and_invisible_decorations() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("--\n\n== Nested\n\n[%step]\n* item\n\n--\n")
            .unwrap();
        let doc = analysis.document();
        let root = doc.index().top_level_blocks()[0];
        let AstBlock::Delimited(compound) = doc.block(root).unwrap() else {
            unreachable!()
        };
        let crate::block_model::DelimitedContent::Compound(children) = &compound.content else {
            unreachable!()
        };
        let heading = doc.index().block_id_at(children[0].range()).unwrap();
        let list = doc.index().block_id_at(children[1].range()).unwrap();
        let region = |blocks| HtmlRegionSelection {
            blocks,
            ..Default::default()
        };
        for selections in [
            vec![region(vec![root, root])],
            vec![region(vec![root]), region(vec![list])],
            vec![region(vec![heading, heading])],
        ] {
            assert!(matches!(
                render_regions(
                    doc,
                    &RenderPolicy::default(),
                    &RenderInputs::default(),
                    &selections,
                    OutputLimits::default()
                ),
                Err(HtmlRegionError::InvalidSelection {
                    reason: "selected roots overlap or repeat a block",
                    ..
                })
            ));
        }
        for selection in [
            HtmlRegionSelection {
                stepped_blocks: [(list, 0)].into(),
                blocks: vec![heading],
                ..Default::default()
            },
            HtmlRegionSelection {
                stepped_blocks: [(list, 0)].into(),
                blocks: vec![root],
                omitted_blocks: [list].into(),
                ..Default::default()
            },
            HtmlRegionSelection {
                container_headings: [heading].into(),
                blocks: vec![list],
                ..Default::default()
            },
        ] {
            assert!(matches!(
                render_regions(
                    doc,
                    &RenderPolicy::default(),
                    &RenderInputs::default(),
                    &[selection],
                    OutputLimits::default()
                ),
                Err(HtmlRegionError::InvalidSelection {
                    reason: "presentation target is outside the region or omitted",
                    ..
                })
            ));
        }
    }

    #[test]
    fn resolutions_share_usage_across_regions_and_omitted_input_reports_its_source_range() {
        let analysis = Engine::new(AnalysisOptions::default()).analyze("xref:note:first[First]\n\nxref:note:second[Second]\n\nxref:note:private[Private]\n").unwrap();
        let doc = analysis.document();
        let roots = doc.index().top_level_blocks();
        let references = analysis.references();
        let inputs = RenderInputs::default().with_references(
            references
                .iter()
                .enumerate()
                .map(|(index, reference)| {
                    crate::reference::ResolvedReference::resolved(
                        reference.range,
                        format!("https://example.test/{index}"),
                    )
                })
                .collect(),
        );
        let selections = [
            HtmlRegionSelection {
                blocks: vec![roots[0]],
                ..Default::default()
            },
            HtmlRegionSelection {
                blocks: vec![roots[1], roots[2]],
                omitted_blocks: [roots[2]].into(),
                ..Default::default()
            },
        ];
        let output = render_regions(
            doc,
            &RenderPolicy::default(),
            &inputs,
            &selections,
            OutputLimits::default(),
        )
        .unwrap();
        assert!(output.regions[0].contains("https://example.test/0"));
        assert!(output.regions[1].contains("https://example.test/1"));
        assert!(!output.regions[1].contains("Private"));
        let unused = output
            .diagnostics
            .iter()
            .filter(|item| item.code.as_str() == "unused-render-input")
            .collect::<Vec<_>>();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].range, references[2].range);
        assert!(!unused[0].message.contains("private"));
    }

    #[test]
    fn nested_omission_and_steps_use_safe_body_rendering() {
        let analysis = Engine::new(AnalysisOptions::default()).analyze("= Title\n\n[%step]\n* <script>unsafe</script>\n* second\n\n--\nPublic.\n\n[.notes]\n====\nSECRET\n====\n--\n").unwrap();
        let doc = analysis.document();
        let roots = doc.index().top_level_blocks();
        let secret = doc
            .blocks()
            .iter()
            .find_map(|block| match block {
                AstBlock::Delimited(block) => match &block.content {
                    crate::block_model::DelimitedContent::Compound(children) => {
                        children.iter().find(|block| {
                            block
                                .metadata()
                                .role_names()
                                .any(|(role, _)| role == "notes")
                        })
                    }
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        let request = HtmlRegionSelection {
            blocks: roots.to_vec(),
            omitted_blocks: [doc.index().block_id_at(secret.range()).unwrap()].into(),
            stepped_blocks: [(roots[1], 0)].into(),
            ..Default::default()
        };
        let output = render_regions(
            doc,
            &RenderPolicy::default(),
            &RenderInputs::default(),
            &[request],
            OutputLimits::default(),
        )
        .unwrap();
        assert!(output.regions[0].contains("&lt;script&gt;unsafe&lt;/script&gt;"));
        assert!(
            output.regions[0].contains("<li class=\"fragment\" data-fragment-index=\"1\">second")
        );
        assert!(output.regions[0].contains("Public."));
        assert!(!output.regions[0].contains("SECRET"));
        let ordinary = super::super::render(doc, &RenderPolicy::default());
        assert!(ordinary.html.contains("SECRET"));
        assert!(!ordinary.html.contains("class=\"fragment\""));
    }

    #[test]
    fn output_limit_counts_all_regions_and_container_heading_has_no_duplicate_id() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("= First\n\n== Second\n")
            .unwrap();
        let doc = analysis.document();
        let roots = doc.index().top_level_blocks();
        let selections = roots
            .iter()
            .map(|&id| HtmlRegionSelection {
                blocks: vec![id],
                container_headings: [id].into(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let output = render_regions(
            doc,
            &RenderPolicy::default(),
            &RenderInputs::default(),
            &selections,
            OutputLimits::default(),
        )
        .unwrap();
        assert!(output.regions.iter().all(|html| !html.contains(" id=")));
        let limit =
            u32::try_from(output.regions.iter().map(String::len).sum::<usize>() - 1).unwrap();
        assert!(matches!(
            render_regions(
                doc,
                &RenderPolicy::default(),
                &RenderInputs::default(),
                &selections,
                OutputLimits {
                    max_output_bytes: limit
                }
            ),
            Err(HtmlRegionError::OutputLimit { .. })
        ));
    }
}
