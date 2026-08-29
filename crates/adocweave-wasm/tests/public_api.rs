use std::collections::BTreeMap;

use adocweave_wasm::{
    ActiveUrlOptions, AnalyzeRequest, AuthoredUrlOptions, CitationOutcome, CitationSegment,
    DiagnosticOptions, ExternalLinkOptions, GeneratedBibliography, GeneratedBibliographyEntry,
    HtmlOptions, ProductRequest, ReferenceNotice, ReferenceOutcome, ResolvedCitation,
    ResolvedReference, ResolvedResource, ResourceCapabilities, ResourceInput, ResourceOutcome,
    RoleOptions, RuleOptions, SourceInput, SourceLanguageOptions, Stylesheet,
};

#[test]
fn public_request_types_are_constructible_with_struct_literals() {
    let source_languages = SourceLanguageOptions {
        allowed: Some(vec!["rust".to_owned()]),
        unknown: None,
    };
    let roles = RoleOptions {
        allowed: Some(vec!["note".to_owned()]),
        unknown: None,
    };
    let resource_capabilities = ResourceCapabilities {
        images: Some(true),
        media: Some(false),
    };
    let html = HtmlOptions {
        document_mode: None,
        active_urls: Some(ActiveUrlOptions {
            allowed_schemes: Some(vec!["https".to_owned()]),
            allow_authored_relative: Some(true),
            allow_resolved_relative: Some(true),
            allow_resolved_root_relative: Some(false),
            allow_data_uris: Some(false),
        }),
        external_links: Some(ExternalLinkOptions {
            open_in_new_context: Some(true),
            noreferrer: Some(true),
        }),
        source_languages: Some(source_languages),
        roles: Some(roles),
        math_languages: None,
        unresolved_references: None,
        resource_capabilities: Some(resource_capabilities),
        stylesheets: Some(vec![Stylesheet::Inline {
            css: "p { color: black; }".to_owned(),
        }]),
    };
    let diagnostics = DiagnosticOptions {
        protected_attributes: Some(BTreeMap::new()),
        authored_urls: Some(AuthoredUrlOptions {
            allowed_schemes: Some(vec!["https".to_owned()]),
            allow_relative: Some(false),
        }),
        rules: Some(BTreeMap::from([(
            "example-rule".to_owned(),
            RuleOptions {
                enabled: Some(true),
                severity: None,
            },
        )])),
        max_diagnostics: Some(10),
    };
    let products = ProductRequest {
        syntax: Some(()),
        canonical_ast: None,
        html: Some(html),
        attribute_occurrences: None,
        attribute_queries: None,
        resource_queries: None,
        diagnostics: Some(diagnostics),
        symbols: None,
        document: None,
    };

    let references = vec![ResolvedReference {
        source_start: 0,
        source_end: 0,
        outcome: ReferenceOutcome::Resolved {
            href: "chapter.adoc".to_owned(),
            display_text: Some("Chapter".to_owned()),
            notices: vec![ReferenceNotice::Fallback],
        },
    }];
    let assets = vec![ResolvedResource {
        source_start: 0,
        source_end: 0,
        outcome: ResourceOutcome::Resolved {
            href: "image.png".to_owned(),
            media_type: "image/png".to_owned(),
            byte_length: Some(42),
        },
    }];
    let citations = vec![ResolvedCitation {
        source_start: 0,
        source_end: 0,
        outcome: CitationOutcome::Resolved {
            segments: vec![CitationSegment {
                text: "Example".to_owned(),
                anchor: Some("example".to_owned()),
            }],
        },
    }];
    let bibliography = GeneratedBibliography {
        title: "References".to_owned(),
        entries: vec![GeneratedBibliographyEntry {
            citation_key: "example".to_owned(),
            text: "Example entry".to_owned(),
            label: Some("[1]".to_owned()),
            number: Some(1),
        }],
    };
    let resources = ResourceInput {
        documents: Some(BTreeMap::from([(
            "chapter.adoc".to_owned(),
            "Chapter".to_owned(),
        )])),
        base_uri: None,
        safe_mode: None,
        allowed_schemes: Some(vec!["https".to_owned()]),
        includes: None,
        references: Some(references),
        assets: Some(assets),
        citations: Some(citations),
        bibliography: Some(bibliography),
    };
    let request = AnalyzeRequest {
        source: SourceInput {
            text: "Text".to_owned(),
            id: Some("main.adoc".to_owned()),
            attributes: Some(BTreeMap::new()),
            syntax_mode: None,
        },
        products,
        resources: Some(resources),
    };

    assert_eq!(request.source.text, "Text");
}
