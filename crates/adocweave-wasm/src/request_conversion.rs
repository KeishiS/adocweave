use std::collections::BTreeSet;

use adocweave_core::output::diagnostics::{LintConfig, RuleSettings, Severity, lint_rule};
use adocweave_core::output::html::{
    ExternalLinkPresentation, HtmlDocumentMode, MathLanguagePolicy, RenderPolicy,
    ResourceCapabilities as CoreResourceCapabilities, RolePolicy, SourceLanguagePolicy,
    StylesheetPolicy, StylesheetSource, UnknownRole as CoreUnknownRole,
    UnknownSourceLanguage as CoreUnknownSourceLanguage,
    UnresolvedReferencePresentation as CoreUnresolvedReferencePresentation,
};
use adocweave_core::preprocess::{EffectiveProcessingOptions, PreprocessOptions, ResourceSnapshot};
use adocweave_core::resolution::{ActiveUrlPolicy, AuthoredUrlPolicy};
use adocweave_core::{
    AnalysisLimits, AnalysisOptions, DiagnosticProfile, SourceId, SyntaxMode as CoreSyntaxMode,
    SyntaxOptions,
};

use crate::preprocess_wire::resource_snapshot;
use crate::render_input_normalization::{self, NormalizedRenderInputs};
use crate::render_input_wire::RenderInputs;
use crate::{
    AdocWeaveError, AnalyzeRequest, DiagnosticOptions, DocumentMode, HtmlOptions, IncludeHandling,
    MathLanguage, ProductRequest, ResourceInput, SafeMode, Stylesheet, SyntaxMode, UnknownRole,
    UnknownSourceLanguage, UnresolvedReferencePresentation,
};

pub(crate) struct ExecutionRequest {
    pub(crate) source: String,
    pub(crate) source_id: Option<SourceId>,
    pub(crate) requested_products: ProductRequest,
    pub(crate) render_inputs: NormalizedRenderInputs,
    pub(crate) snapshot: ResourceSnapshot,
    pub(crate) processing_options: EffectiveProcessingOptions,
    pub(crate) render_policy: RenderPolicy,
    pub(crate) max_output_bytes: usize,
}

pub(crate) fn convert(request: AnalyzeRequest) -> Result<ExecutionRequest, AdocWeaveError> {
    if request.products.is_empty() {
        return Err(invalid_request("products must request at least one result"));
    }
    validate_identifier(request.source.id.as_deref(), "source.id")?;
    validate_attribute_names(request.source.attributes.as_ref())?;
    validate_product_options(&request.products)?;

    let source_id = request.source.id.map(SourceId::new);
    let resources = request.resources.unwrap_or_default();
    validate_resources(&resources)?;

    let diagnostics = diagnostic_profile(request.products.diagnostics.as_ref())?;
    let analysis_limits = AnalysisLimits::default();
    let analysis_options = AnalysisOptions {
        syntax: SyntaxOptions {
            syntax_mode: match request.source.syntax_mode.unwrap_or_default() {
                SyntaxMode::Permissive => CoreSyntaxMode::Permissive,
                SyntaxMode::Strict => CoreSyntaxMode::Strict,
            },
            limits: analysis_limits,
        },
        diagnostics,
        attributes: request.source.attributes.unwrap_or_default(),
    };

    let preprocess_options = PreprocessOptions {
        source_id: source_id.clone(),
        base_uri: resources.base_uri,
        safe_mode: match resources.safe_mode.unwrap_or_default() {
            SafeMode::Unsafe => adocweave_core::preprocess::SafeMode::Unsafe,
            SafeMode::Server => adocweave_core::preprocess::SafeMode::Server,
            SafeMode::Safe => adocweave_core::preprocess::SafeMode::Safe,
            SafeMode::Secure => adocweave_core::preprocess::SafeMode::Secure,
        },
        allowed_schemes: resources
            .allowed_schemes
            .unwrap_or_default()
            .into_iter()
            .map(|scheme| scheme.to_ascii_lowercase())
            .collect(),
        attributes: analysis_options.attributes.clone(),
        enable_includes: !matches!(resources.includes, Some(IncludeHandling::Preserve)),
        max_attribute_expansion_depth: analysis_options.syntax.limits.max_attribute_expansion_depth,
        max_attribute_expansion_bytes: analysis_options.syntax.limits.max_attribute_expansion_bytes,
        ..PreprocessOptions::default()
    };
    let processing_options = EffectiveProcessingOptions::new(analysis_options, preprocess_options)
        .map_err(|error| invalid_request(error.to_string()))?;

    let documents = resources.documents.unwrap_or_default();
    let render_inputs = RenderInputs {
        references: resources.references.unwrap_or_default(),
        resources: resources.assets.unwrap_or_default(),
        citations: resources.citations.unwrap_or_default(),
        generated_bibliography: resources.bibliography,
    };
    let output_limits = adocweave_core::OutputLimits::default();
    let render_inputs = render_input_normalization::normalize(
        render_inputs,
        &documents,
        &request.source.text,
        &analysis_limits,
    )?;
    let render_policy = render_policy(request.products.html.as_ref())?;
    let snapshot = resource_snapshot(documents);

    Ok(ExecutionRequest {
        source: request.source.text,
        source_id,
        requested_products: request.products,
        render_inputs,
        snapshot,
        processing_options,
        render_policy,
        max_output_bytes: usize::try_from(output_limits.max_output_bytes)
            .expect("u32 fits usize on supported targets"),
    })
}

fn diagnostic_profile(
    options: Option<&DiagnosticOptions>,
) -> Result<DiagnosticProfile, AdocWeaveError> {
    let Some(options) = options else {
        return Ok(DiagnosticProfile::default());
    };
    let mut lint = LintConfig::default();
    lint.protected_attributes = options.protected_attributes.clone().unwrap_or_default();
    if let Some(authored_urls) = &options.authored_urls {
        let defaults = AuthoredUrlPolicy::default();
        lint.authored_url_policy = AuthoredUrlPolicy {
            allowed_schemes: authored_urls
                .allowed_schemes
                .clone()
                .unwrap_or_else(|| defaults.allowed_schemes.into_iter().collect())
                .into_iter()
                .map(|scheme| scheme.to_ascii_lowercase())
                .collect(),
            allow_relative: authored_urls
                .allow_relative
                .unwrap_or(defaults.allow_relative),
        };
    }
    lint.max_diagnostics = usize::try_from(options.max_diagnostics.unwrap_or(1000))
        .expect("u32 fits usize on supported targets");
    if lint.max_diagnostics > 1000 {
        return Err(invalid_request("maxDiagnostics must be between 0 and 1000"));
    }
    for (code, settings) in options.rules.clone().unwrap_or_default() {
        let Some(descriptor) = lint_rule(&code) else {
            return Err(invalid_request(format!("unknown lint rule: {code}")));
        };
        lint.set_rule(
            descriptor.id,
            RuleSettings {
                enabled: settings.enabled.unwrap_or(true),
                severity: match settings.severity.unwrap_or(crate::Severity::Warning) {
                    crate::Severity::Error => Severity::Error,
                    crate::Severity::Warning => Severity::Warning,
                    crate::Severity::Information => Severity::Information,
                    crate::Severity::Hint => Severity::Hint,
                },
            },
        );
    }
    Ok(DiagnosticProfile { lint })
}

fn render_policy(options: Option<&HtmlOptions>) -> Result<RenderPolicy, AdocWeaveError> {
    let Some(options) = options else {
        return Ok(RenderPolicy::default());
    };
    let defaults = RenderPolicy::default();
    let active_urls = options.active_urls.as_ref().map_or_else(
        || defaults.active_urls.clone(),
        |options| {
            let policy_defaults = ActiveUrlPolicy::default();
            ActiveUrlPolicy {
                allowed_schemes: options
                    .allowed_schemes
                    .clone()
                    .unwrap_or_else(|| policy_defaults.allowed_schemes.into_iter().collect())
                    .into_iter()
                    .map(|scheme| scheme.to_ascii_lowercase())
                    .collect::<BTreeSet<_>>(),
                allow_authored_relative: options
                    .allow_authored_relative
                    .unwrap_or(policy_defaults.allow_authored_relative),
                allow_resolved_relative: options
                    .allow_resolved_relative
                    .unwrap_or(policy_defaults.allow_resolved_relative),
                allow_resolved_root_relative: options
                    .allow_resolved_root_relative
                    .unwrap_or(policy_defaults.allow_resolved_root_relative),
                allow_data_uris: options
                    .allow_data_uris
                    .unwrap_or(policy_defaults.allow_data_uris),
            }
        },
    );
    let external_links = options.external_links.as_ref().map_or_else(
        || defaults.external_links,
        |options| {
            if options.open_in_new_context.unwrap_or(false) {
                ExternalLinkPresentation::NewContext {
                    noreferrer: options.noreferrer.unwrap_or(false),
                }
            } else {
                ExternalLinkPresentation::SameContext
            }
        },
    );
    let source_languages = options.source_languages.as_ref().map_or_else(
        || defaults.source_languages.clone(),
        |options| SourceLanguagePolicy {
            allowed: options.allowed.clone().map(|languages| {
                languages
                    .into_iter()
                    .map(|language| language.to_ascii_lowercase())
                    .collect()
            }),
            unknown: match options.unknown.unwrap_or_default() {
                UnknownSourceLanguage::PreserveSanitized => {
                    CoreUnknownSourceLanguage::PreserveSanitized
                }
                UnknownSourceLanguage::OmitClass => CoreUnknownSourceLanguage::OmitClass,
                UnknownSourceLanguage::Diagnostic => CoreUnknownSourceLanguage::Diagnostic,
            },
        },
    );
    let roles = options.roles.as_ref().map_or_else(
        || defaults.roles.clone(),
        |options| RolePolicy {
            allowed: options
                .allowed
                .clone()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            unknown: match options.unknown.unwrap_or_default() {
                UnknownRole::Silent => CoreUnknownRole::Silent,
                UnknownRole::Diagnostic => CoreUnknownRole::Diagnostic,
            },
        },
    );
    let math_languages = MathLanguagePolicy {
        allowed: options
            .math_languages
            .clone()
            .unwrap_or_else(|| vec![MathLanguage::Latex, MathLanguage::Typst])
            .into_iter()
            .map(|language| match language {
                MathLanguage::Latex => adocweave_core::semantic::MathLanguage::Latex,
                MathLanguage::Typst => adocweave_core::semantic::MathLanguage::Typst,
            })
            .collect(),
    };
    let resource_defaults = defaults.resources;
    let resources = options
        .resource_capabilities
        .as_ref()
        .map_or(resource_defaults, |options| CoreResourceCapabilities {
            images: options.images.unwrap_or(resource_defaults.images),
            media: options.media.unwrap_or(resource_defaults.media),
        });
    let stylesheets = options.stylesheets.clone().unwrap_or_default();
    Ok(RenderPolicy {
        active_urls,
        external_links,
        source_languages,
        roles,
        math_languages,
        unresolved_references: match options.unresolved_references.unwrap_or_default() {
            UnresolvedReferencePresentation::Target => CoreUnresolvedReferencePresentation::Target,
            UnresolvedReferencePresentation::LabelOnly => {
                CoreUnresolvedReferencePresentation::LabelOnly
            }
            UnresolvedReferencePresentation::Hidden => CoreUnresolvedReferencePresentation::Hidden,
        },
        resources,
        document_mode: match options.document_mode.unwrap_or_default() {
            DocumentMode::Fragment => HtmlDocumentMode::Fragment,
            DocumentMode::Complete => HtmlDocumentMode::Complete,
        },
        stylesheets: StylesheetPolicy {
            sources: stylesheets
                .into_iter()
                .map(|stylesheet| match stylesheet {
                    Stylesheet::Inline { css } => StylesheetSource::Inline(css),
                    Stylesheet::External { url } => StylesheetSource::External(url),
                })
                .collect(),
            ..StylesheetPolicy::default()
        },
        ..defaults
    })
}

fn validate_identifier(value: Option<&str>, field: &str) -> Result<(), AdocWeaveError> {
    if value.is_some_and(|value| value.is_empty() || value.chars().any(char::is_control)) {
        return Err(invalid_request(format!(
            "{field} must be non-empty and control-free"
        )));
    }
    Ok(())
}

fn validate_attribute_names(
    attributes: Option<&std::collections::BTreeMap<String, Option<String>>>,
) -> Result<(), AdocWeaveError> {
    if attributes.is_some_and(|attributes| {
        attributes
            .keys()
            .any(|name| name.is_empty() || name.chars().any(char::is_control))
    }) {
        return Err(invalid_request(
            "attribute names must be non-empty and control-free",
        ));
    }
    Ok(())
}

fn validate_resources(resources: &ResourceInput) -> Result<(), AdocWeaveError> {
    if resources.documents.as_ref().is_some_and(|documents| {
        documents
            .keys()
            .any(|name| name.is_empty() || name.chars().any(char::is_control))
    }) {
        return Err(invalid_request(
            "resource document IDs must be non-empty and control-free",
        ));
    }
    if let Some(base_uri) = &resources.base_uri {
        validate_absolute_uri(base_uri, "resources.baseUri")?;
    }
    validate_schemes(
        resources.allowed_schemes.as_deref(),
        "resources.allowedSchemes",
    )?;
    Ok(())
}

fn validate_product_options(products: &ProductRequest) -> Result<(), AdocWeaveError> {
    if let Some(diagnostics) = &products.diagnostics {
        validate_schemes(
            diagnostics
                .authored_urls
                .as_ref()
                .and_then(|options| options.allowed_schemes.as_deref()),
            "products.diagnostics.authoredUrls.allowedSchemes",
        )?;
    }
    if let Some(html) = &products.html {
        validate_schemes(
            html.active_urls
                .as_ref()
                .and_then(|options| options.allowed_schemes.as_deref()),
            "products.html.activeUrls.allowedSchemes",
        )?;
    }
    Ok(())
}

fn validate_schemes(schemes: Option<&[String]>, field: &str) -> Result<(), AdocWeaveError> {
    let Some(schemes) = schemes else {
        return Ok(());
    };
    let mut unique = BTreeSet::new();
    for scheme in schemes {
        let normalized = scheme.to_ascii_lowercase();
        let mut characters = scheme.chars();
        let valid = characters
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
            && characters.all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
            });
        if !valid || !unique.insert(normalized) {
            return Err(invalid_request(format!(
                "{field} must contain valid, unique URI schemes"
            )));
        }
    }
    Ok(())
}

fn validate_absolute_uri(value: &str, field: &str) -> Result<(), AdocWeaveError> {
    let Some((scheme, remainder)) = value.split_once(':') else {
        return Err(invalid_request(format!("{field} must be an absolute URI")));
    };
    validate_schemes(Some(&[scheme.to_owned()]), field)?;
    if remainder.is_empty() || value.chars().any(char::is_control) {
        return Err(invalid_request(format!("{field} must be an absolute URI")));
    }
    Ok(())
}

pub(crate) fn invalid_request(message: impl Into<String>) -> AdocWeaveError {
    AdocWeaveError {
        code: "invalid-request".to_owned(),
        message: message.into(),
    }
}
