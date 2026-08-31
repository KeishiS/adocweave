use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use adocweave_core::output::html::{
    HtmlDocumentMode, HtmlOutput, RenderPolicy, StylesheetPolicy, StylesheetSource, render,
};
use adocweave_core::semantic::Document;

/// A stylesheet argument in command-line order. Files are embedded and URLs
/// are linked; both apply only to complete document output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StylesheetArgument {
    File(PathBuf),
    Url(String),
}

#[derive(Debug)]
pub(crate) enum Error {
    Read {
        source_name: String,
        source: io::Error,
    },
    ProjectLimit(adocweave_project::ProjectLimit),
    Stylesheet(String),
    Usage(String),
}

pub(crate) fn validate_argument_count(stylesheets: &[StylesheetArgument]) -> Result<(), Error> {
    let limit = usize::try_from(StylesheetPolicy::default().max_sources).unwrap_or(usize::MAX);
    if stylesheets.len() > limit {
        return Err(Error::Stylesheet(format!(
            "stylesheet count exceeds the limit of {limit}"
        )));
    }
    Ok(())
}

pub(crate) fn build_project(
    project: &adocweave_project::ProjectConfig,
    resources: &[adocweave_project::ProjectResourceResult],
    complete: bool,
    stylesheets: &[StylesheetArgument],
) -> Result<RenderPolicy, Error> {
    validate_argument_count(stylesheets)?;
    let limits = StylesheetPolicy::default();
    let command_file_count = stylesheets
        .iter()
        .filter(|value| matches!(value, StylesheetArgument::File(_)))
        .count();
    let configured_file_count = project
        .stylesheet_files()
        .len()
        .checked_sub(command_file_count)
        .ok_or_else(|| Error::Stylesheet("stylesheet result is incomplete".to_owned()))?;
    let count = project
        .stylesheet_files()
        .len()
        .saturating_add(project.stylesheet_urls().len())
        .saturating_add(stylesheets.len().saturating_sub(command_file_count));
    if count > usize::try_from(limits.max_sources).unwrap_or(usize::MAX) {
        return Err(Error::Stylesheet(format!(
            "stylesheet count exceeds the limit of {}",
            limits.max_sources
        )));
    }
    let inline = |path: &Path| -> Result<StylesheetSource, Error> {
        let resource = resources
            .iter()
            .find(|resource| {
                resource.kind == adocweave_project::ProjectResourceKind::Stylesheet
                    && (resource.requested_path == path || resource.path == path)
            })
            .ok_or_else(|| Error::Read {
                source_name: path.display().to_string(),
                source: io::Error::other("stylesheet was not returned by project processing"),
            })?;
        let source = match &resource.outcome {
            adocweave_project::ProjectResourceOutcome::Loaded { source } => source,
            adocweave_project::ProjectResourceOutcome::LoadedOmitted { limit } => {
                return Err(Error::ProjectLimit(*limit));
            }
            adocweave_project::ProjectResourceOutcome::Failed(
                adocweave_project::ProjectResourceFailure::Limit(limit),
            ) => {
                return Err(Error::ProjectLimit(*limit));
            }
            outcome => {
                return Err(Error::Read {
                    source_name: path.display().to_string(),
                    source: io::Error::other(format!("stylesheet is unavailable: {outcome:?}")),
                });
            }
        };
        if source.len()
            > usize::try_from(limits.max_inline_bytes).expect("u32 fits usize on supported targets")
        {
            return Err(Error::Stylesheet(format!(
                "stylesheet {} exceeds the limit of {} bytes",
                path.display(),
                limits.max_inline_bytes
            )));
        }
        Ok(StylesheetSource::Inline(source.to_string()))
    };

    let mut sources = project.stylesheet_files()[..configured_file_count]
        .iter()
        .map(|path| inline(path))
        .collect::<Result<Vec<_>, _>>()?;
    sources.extend(
        project
            .stylesheet_urls()
            .iter()
            .cloned()
            .map(StylesheetSource::External),
    );
    let mut command_files = project.stylesheet_files()[configured_file_count..].iter();
    for stylesheet in stylesheets {
        match stylesheet {
            StylesheetArgument::File(_) => {
                let path = command_files.next().ok_or_else(|| {
                    Error::Stylesheet("stylesheet result is incomplete".to_owned())
                })?;
                sources.push(inline(path)?);
            }
            StylesheetArgument::Url(url) => {
                sources.push(StylesheetSource::External(url.clone()));
            }
        }
    }
    let mut policy = project.html_policy().clone();
    if complete {
        policy.document_mode = HtmlDocumentMode::Complete;
    }
    if policy.document_mode != HtmlDocumentMode::Complete && !sources.is_empty() {
        return Err(Error::Usage(
            "--css and --css-url require --complete".to_owned(),
        ));
    }
    policy.stylesheets = StylesheetPolicy { sources, ..limits };
    Ok(policy)
}

pub(crate) fn render_checked(
    document: &Document,
    policy: &RenderPolicy,
) -> Result<HtmlOutput, Error> {
    let output = render(document, policy);
    if let Some(diagnostic) = output
        .diagnostics
        .iter()
        .find(|diagnostic| is_stylesheet_error(diagnostic.code.as_str()))
    {
        return Err(Error::Stylesheet(diagnostic.message.clone()));
    }
    Ok(output)
}

pub(crate) fn external_origins(policy: &RenderPolicy) -> BTreeSet<String> {
    policy
        .stylesheets
        .sources
        .iter()
        .filter_map(|source| match source {
            StylesheetSource::External(value) => url::Url::parse(value).ok(),
            StylesheetSource::Inline(_) => None,
        })
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.origin().ascii_serialization())
        .collect()
}

fn is_stylesheet_error(code: &str) -> bool {
    matches!(
        code,
        "invalid-stylesheet-url"
            | "invalid-stylesheet-content"
            | "stylesheet-limit-exceeded"
            | "stylesheet-not-applicable"
    )
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read {
                source_name,
                source,
            } => write!(formatter, "could not read {source_name}: {source}"),
            Self::ProjectLimit(limit) => limit.fmt(formatter),
            Self::Stylesheet(message) | Self::Usage(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use adocweave_core::{AnalysisOptions, Engine};

    use super::*;

    #[test]
    fn checked_render_promotes_the_first_stylesheet_diagnostic() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("body\n")
            .expect("analysis");
        let policy = RenderPolicy {
            document_mode: HtmlDocumentMode::Complete,
            stylesheets: StylesheetPolicy {
                sources: vec![StylesheetSource::External("javascript:alert(1)".to_owned())],
                ..StylesheetPolicy::default()
            },
            ..RenderPolicy::default()
        };

        let error =
            render_checked(analysis.document(), &policy).expect_err("invalid stylesheet URL");

        assert!(matches!(error, Error::Stylesheet(message) if message.contains("stylesheet URL")));
    }
}
