use adocweave_core::output::diagnostics::{LINT_RULES, LintRuleDescriptor, LintRuleId};
use adocweave_core::output::html::RenderPolicy;
use adocweave_core::resolution::{ActiveUrlPolicy, AuthoredUrlPolicy, UrlProvenance};
use adocweave_core::{
    AnalysisLimits, AnalysisOptions, DiagnosticProfile, Engine, OutputLimits, SyntaxOptions,
};

#[test]
fn responsibility_specific_configuration_is_publicly_importable() {
    let analysis_options = AnalysisOptions {
        syntax: SyntaxOptions {
            limits: AnalysisLimits::default(),
            ..SyntaxOptions::default()
        },
        diagnostics: DiagnosticProfile::default(),
        attributes: Default::default(),
    };
    let analysis = Engine::new(analysis_options)
        .analyze("link:https://example.com[example]")
        .expect("analysis");
    let policy = RenderPolicy {
        active_urls: ActiveUrlPolicy::default(),
        ..RenderPolicy::default()
    };

    assert!(policy.allows_url("https://example.com", UrlProvenance::Authored));
    assert!(!analysis.document().blocks().is_empty());
    assert!(AuthoredUrlPolicy::default().allows("guide.adoc"));
    assert!(OutputLimits::default().max_output_bytes > 0);

    let descriptor: &LintRuleDescriptor = &LINT_RULES[0];
    let id: LintRuleId = descriptor.id;
    assert!(!id.as_str().is_empty());
}

#[test]
fn release_note_configuration_example_uses_public_paths() {
    let mut lint = adocweave_core::output::diagnostics::LintConfig::default();
    lint.authored_url_policy.allow_relative = true;
    let _engine = adocweave_core::Engine::new(adocweave_core::AnalysisOptions {
        diagnostics: adocweave_core::DiagnosticProfile { lint },
        ..adocweave_core::AnalysisOptions::default()
    });
    let policy = adocweave_core::output::html::RenderPolicy {
        active_urls: adocweave_core::resolution::ActiveUrlPolicy {
            allow_authored_relative: true,
            ..adocweave_core::resolution::ActiveUrlPolicy::default()
        },
        ..adocweave_core::output::html::RenderPolicy::default()
    };

    assert!(policy.active_urls.allow_authored_relative);
}

#[test]
fn projection_product_field_types_are_publicly_nameable() {
    let _: Option<adocweave_core::preprocess::ProjectedAttributeBinding> = None;
    let _: Option<adocweave_core::preprocess::ProjectedAttributeReference> = None;
    let _: Option<adocweave_core::preprocess::ProjectedDiagnostic> = None;
    let _: Option<adocweave_core::preprocess::ProjectedDocumentAttribute> = None;
    let _: Option<adocweave_core::preprocess::ProjectedDocumentAttributeValueLine> = None;
    let _: Option<adocweave_core::preprocess::ProjectedDocumentSymbol> = None;
    let _: Option<adocweave_core::preprocess::ProjectedFix> = None;
    let _: Option<adocweave_core::preprocess::ProjectedLocalTarget> = None;
    let _: Option<adocweave_core::preprocess::ProjectedReference> = None;
    let _: Option<adocweave_core::preprocess::ProjectedResource> = None;
}

#[test]
fn focused_projection_queries_are_publicly_nameable() {
    use adocweave_core::output::projection::{
        BlockPresentationProjection, ExternalLink, FormulaProjection, OrderedListProjection,
        ProjectedText, ReferenceEdge, RenderingFeatures, SearchableText, SourceBlockProjection,
        block_presentations, document_title, external_links, formulas, ordered_lists,
        reference_edges, rendering_features, searchable_text, source_blocks,
    };

    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("= Title\n\nhttps://example.com[]\n")
        .expect("analysis");
    let inputs = adocweave_core::resolution::RenderInputs::default();

    let _: Option<ProjectedText> = document_title(&analysis);
    let _: Vec<ExternalLink> = external_links(&analysis);
    let _: Vec<ReferenceEdge> = reference_edges(&analysis, &inputs);
    let _: Vec<SourceBlockProjection> = source_blocks(&analysis);
    let _: Vec<FormulaProjection> = formulas(&analysis);
    let _: Vec<OrderedListProjection> = ordered_lists(&analysis);
    let _: Vec<BlockPresentationProjection> = block_presentations(&analysis);
    let _: SearchableText = searchable_text(&analysis);
    let _: RenderingFeatures = rendering_features(&analysis);
}

#[test]
fn cancellable_lint_api_is_public() {
    let analysis = Engine::new(AnalysisOptions::default())
        .analyze("paragraph\n")
        .expect("analysis");
    let cancellation = adocweave_core::CancellationToken::new();
    cancellation.cancel();

    assert_eq!(
        adocweave_core::output::diagnostics::lint_analysis(
            &analysis,
            &adocweave_core::output::diagnostics::LintConfig::default(),
            &cancellation,
        )
        .expect_err("cancelled lint"),
        adocweave_core::output::diagnostics::LintError::Cancelled
    );
}
