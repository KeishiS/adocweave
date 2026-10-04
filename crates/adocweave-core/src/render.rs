//! Backend-neutral, fully resolved inputs consumed by pure renderers.

use std::collections::BTreeMap;

use crate::citation::ResolvedCitation;
use crate::generated_bibliography::GeneratedBibliography;
use crate::reference::ResolvedReference;
use crate::rendered_content::{ResolvedMath, ResolvedRichCitation};
use crate::resource::ResolvedResource;
use crate::source::TextRange;

/// An owned snapshot of every host resolution result used during rendering.
///
/// Construction performs no I/O. Exact-range indexes are built once so every
/// consumer observes the same order-independent duplicate semantics.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RenderInputs {
    references: ResolutionSet<ResolvedReference>,
    resources: ResolutionSet<ResolvedResource>,
    citations: ResolutionSet<ResolvedCitation>,
    math: ResolutionSet<ResolvedMath>,
    rich_citations: ResolutionSet<ResolvedRichCitation>,
    generated_bibliography: Option<GeneratedBibliography>,
}

impl RenderInputs {
    /// Adds the reference resolutions, replacing any set earlier.
    #[must_use]
    pub fn with_references(mut self, references: Vec<ResolvedReference>) -> Self {
        self.references = ResolutionSet::new(references, |resolution| resolution.source_range);
        self
    }

    /// Adds the resource resolutions, replacing any set earlier.
    #[must_use]
    pub fn with_resources(mut self, resources: Vec<ResolvedResource>) -> Self {
        self.resources = ResolutionSet::new(resources, |resolution| resolution.source_range);
        self
    }

    /// Adds the citation resolutions, replacing any set earlier.
    #[must_use]
    pub fn with_citations(mut self, citations: Vec<ResolvedCitation>) -> Self {
        self.citations = ResolutionSet::new(citations, |resolution| resolution.source_range);
        self
    }

    /// Adds validated host-rendered math at exact source ranges.
    #[must_use]
    pub fn with_math(mut self, math: Vec<ResolvedMath>) -> Self {
        self.math = ResolutionSet::new(math, |resolution| resolution.source_range);
        self
    }
    /// Adds finite, validated citation formatting at exact source ranges.
    #[must_use]
    pub fn with_rich_citations(mut self, citations: Vec<ResolvedRichCitation>) -> Self {
        self.rich_citations = ResolutionSet::new(citations, |resolution| resolution.source_range);
        self
    }
    pub fn math(&self) -> &[ResolvedMath] {
        &self.math.values
    }
    pub fn rich_citations(&self) -> &[ResolvedRichCitation] {
        &self.rich_citations.values
    }

    /// Adds a bibliography section whose strings remain plain text.
    #[must_use]
    pub fn with_generated_bibliography(mut self, bibliography: GeneratedBibliography) -> Self {
        self.generated_bibliography = Some(bibliography);
        self
    }

    pub fn references(&self) -> &[ResolvedReference] {
        &self.references.values
    }

    pub fn resources(&self) -> &[ResolvedResource] {
        &self.resources.values
    }

    pub fn citations(&self) -> &[ResolvedCitation] {
        &self.citations.values
    }

    pub fn generated_bibliography(&self) -> Option<&GeneratedBibliography> {
        self.generated_bibliography.as_ref()
    }

    pub fn reference_at(&self, range: TextRange) -> ResolutionMatch<'_, ResolvedReference> {
        self.references.at(range)
    }

    pub fn resource_at(&self, range: TextRange) -> ResolutionMatch<'_, ResolvedResource> {
        self.resources.at(range)
    }

    pub fn citation_at(&self, range: TextRange) -> ResolutionMatch<'_, ResolvedCitation> {
        self.citations.at(range)
    }

    pub fn track_usage(&self) -> RenderInputUsage<'_> {
        RenderInputUsage {
            references: self.references.track(),
            resources: self.resources.track(),
            citations: self.citations.track(),
            math: self.math.track(),
            rich_citations: self.rich_citations.track(),
        }
    }
}

/// Host resolutions of one kind, indexed by the source range they answer.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolutionSet<T> {
    values: Vec<T>,
    index: BTreeMap<TextRange, Vec<usize>>,
}

// Derived `Default` would demand `T: Default`, which no resolution type owes.
impl<T> Default for ResolutionSet<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            index: BTreeMap::new(),
        }
    }
}

impl<T> ResolutionSet<T> {
    fn new(values: Vec<T>, range: impl Fn(&T) -> TextRange) -> Self {
        let mut index = BTreeMap::<_, Vec<usize>>::new();
        for (position, value) in values.iter().enumerate() {
            index.entry(range(value)).or_default().push(position);
        }
        Self { values, index }
    }

    fn at(&self, range: TextRange) -> ResolutionMatch<'_, T> {
        match self.index.get(&range).map(Vec::as_slice) {
            None | Some([]) => ResolutionMatch::Missing,
            Some([position]) => ResolutionMatch::Unique(&self.values[*position]),
            Some(_) => ResolutionMatch::Duplicate,
        }
    }

    fn track(&self) -> UsageTracker<'_, T> {
        UsageTracker {
            set: self,
            used: vec![false; self.values.len()],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionMatch<'a, T> {
    Missing,
    Unique(&'a T),
    Duplicate,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RenderInputDomain {
    Reference,
    Resource,
    Citation,
    Math,
    RichCitation,
}

impl RenderInputDomain {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Resource => "resource",
            Self::Citation => "citation",
            Self::Math => "math",
            Self::RichCitation => "rich citation",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RenderInputProblemKind {
    Duplicate,
    Unused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderInputProblem {
    pub kind: RenderInputProblemKind,
    pub domain: RenderInputDomain,
    pub range: TextRange,
}

pub struct RenderInputUsage<'a> {
    references: UsageTracker<'a, ResolvedReference>,
    resources: UsageTracker<'a, ResolvedResource>,
    citations: UsageTracker<'a, ResolvedCitation>,
    math: UsageTracker<'a, ResolvedMath>,
    rich_citations: UsageTracker<'a, ResolvedRichCitation>,
}

impl<'a> RenderInputUsage<'a> {
    pub fn reference_at(&mut self, range: TextRange) -> ResolutionMatch<'a, ResolvedReference> {
        self.references.at(range)
    }

    pub fn resource_at(&mut self, range: TextRange) -> ResolutionMatch<'a, ResolvedResource> {
        self.resources.at(range)
    }

    pub fn citation_at(&mut self, range: TextRange) -> ResolutionMatch<'a, ResolvedCitation> {
        self.citations.at(range)
    }

    pub fn math_at(&mut self, range: TextRange) -> ResolutionMatch<'a, ResolvedMath> {
        self.math.at(range)
    }
    pub fn rich_citation_at(
        &mut self,
        range: TextRange,
    ) -> ResolutionMatch<'a, ResolvedRichCitation> {
        self.rich_citations.at(range)
    }

    pub fn finish(self) -> Vec<RenderInputProblem> {
        let mut problems = Vec::new();
        self.references.finish(
            RenderInputDomain::Reference,
            |resolution| resolution.source_range,
            &mut problems,
        );
        self.resources.finish(
            RenderInputDomain::Resource,
            |resolution| resolution.source_range,
            &mut problems,
        );
        self.citations.finish(
            RenderInputDomain::Citation,
            |resolution| resolution.source_range,
            &mut problems,
        );
        self.math.finish(
            RenderInputDomain::Math,
            |resolution| resolution.source_range,
            &mut problems,
        );
        self.rich_citations.finish(
            RenderInputDomain::RichCitation,
            |resolution| resolution.source_range,
            &mut problems,
        );
        problems.sort_by_key(|problem| {
            (
                problem.range.start(),
                problem.range.end(),
                problem.domain,
                problem.kind,
            )
        });
        problems
    }
}

/// Records which resolutions of one kind the renderer consumed.
struct UsageTracker<'a, T> {
    set: &'a ResolutionSet<T>,
    used: Vec<bool>,
}

impl<'a, T> UsageTracker<'a, T> {
    fn at(&mut self, range: TextRange) -> ResolutionMatch<'a, T> {
        if let Some(positions) = self.set.index.get(&range) {
            for position in positions {
                self.used[*position] = true;
            }
        }
        self.set.at(range)
    }

    /// A resolution the renderer never asked for, and a range answered more than
    /// once, are both host mistakes. Duplicates count as consumed so the same
    /// resolution is not also reported as unused.
    fn finish(
        mut self,
        domain: RenderInputDomain,
        range: impl Fn(&T) -> TextRange,
        problems: &mut Vec<RenderInputProblem>,
    ) {
        for (duplicated, positions) in &self.set.index {
            if positions.len() < 2 {
                continue;
            }
            for position in positions {
                self.used[*position] = true;
            }
            problems.push(RenderInputProblem {
                kind: RenderInputProblemKind::Duplicate,
                domain,
                range: *duplicated,
            });
        }
        for (resolution, used) in self.set.values.iter().zip(self.used) {
            if !used {
                problems.push(RenderInputProblem {
                    kind: RenderInputProblemKind::Unused,
                    domain,
                    range: range(resolution),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::source::{TextRange, TextSize};

    use super::*;

    #[test]
    fn indexes_are_order_independent_and_usage_is_audited_once() {
        let range = TextRange::new(TextSize::ZERO, TextSize::new(1).expect("size")).expect("range");
        let reference = ResolvedReference::resolved(range, "https://example/reference");
        let inputs = RenderInputs::default()
            .with_references(vec![reference.clone(), reference])
            .with_resources(vec![ResolvedResource::resolved(
                range,
                "https://example/image.png",
                "image/png".parse().expect("media type"),
                Some(42),
            )]);

        assert_eq!(inputs.references().len(), 2);
        assert!(matches!(
            inputs.reference_at(range),
            ResolutionMatch::Duplicate
        ));
        let mut usage = inputs.track_usage();
        assert!(matches!(
            usage.resource_at(range),
            ResolutionMatch::Unique(_)
        ));
        assert_eq!(
            usage.finish(),
            [RenderInputProblem {
                kind: RenderInputProblemKind::Duplicate,
                domain: RenderInputDomain::Reference,
                range,
            }]
        );
    }

    #[test]
    fn every_domain_reports_unused_resolutions_the_renderer_never_asked_for() {
        let range = TextRange::new(TextSize::ZERO, TextSize::new(1).expect("size")).expect("range");
        let inputs = RenderInputs::default().with_citations(vec![ResolvedCitation::resolved(
            range,
            vec![crate::citation::CitationSegment::text("(Smith 2024)")],
        )]);

        assert_eq!(
            inputs.track_usage().finish(),
            [RenderInputProblem {
                kind: RenderInputProblemKind::Unused,
                domain: RenderInputDomain::Citation,
                range,
            }]
        );
    }
}
