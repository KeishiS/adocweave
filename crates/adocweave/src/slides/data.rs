//! Explicit local data files for the optional slides helper.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adocweave_core::CancellationCheck;
use adocweave_project::{ProjectAuthority, ProjectBinaryResource, ProjectResourceLimits};

use super::helper::{Csl, Macro};
use crate::arguments::SlidesData;
use crate::cli_error::CliError;

pub(super) struct DataInputs {
    pub(super) macros: Option<Vec<Macro>>,
    pub(super) csl: Option<Csl>,
    pub(super) resources: Vec<ProjectBinaryResource>,
    pub(super) remaining: ProjectResourceLimits,
}

fn json<T: serde::de::DeserializeOwned>(
    resource: &ProjectBinaryResource,
    purpose: &str,
) -> Result<T, CliError> {
    serde_json::from_slice(&resource.bytes).map_err(|error| {
        CliError::Slides(format!(
            "{}:{}:{}: invalid {purpose} JSON: {error}",
            resource.path.display(),
            error.line(),
            error.column()
        ))
    })
}

fn xml(resource: &ProjectBinaryResource, element: &str) -> Result<String, CliError> {
    let text = std::str::from_utf8(&resource.bytes).map_err(|_| {
        CliError::Slides(format!(
            "{}: {element} XML must be UTF-8",
            resource.path.display()
        ))
    })?;
    if text.len() > 512 * 1024 || text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err(CliError::Slides(format!(
            "{}: {element} XML exceeds limits or contains an unsupported document type",
            resource.path.display()
        )));
    }
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 16_384,
            ..Default::default()
        },
    )
    .map_err(|error| {
        CliError::Slides(format!(
            "{}:{}:{}: invalid {element} XML: {error}",
            resource.path.display(),
            error.pos().row,
            error.pos().col
        ))
    })?;
    let root = document.root_element();
    if root.tag_name().name() != element
        || root.tag_name().namespace() != Some("http://purl.org/net/xbiblio/csl")
        || root.descendants().any(|node| node.ancestors().count() > 64)
    {
        return Err(CliError::Slides(format!(
            "{}: expected a bounded CSL {element} XML document",
            resource.path.display()
        )));
    }
    Ok(text.to_owned())
}

pub(super) fn load(
    options: &SlidesData,
    math: bool,
    citations: bool,
    authority: &ProjectAuthority,
    roots: &[PathBuf],
    limits: ProjectResourceLimits,
    cancellation: &dyn CancellationCheck,
) -> Result<DataInputs, CliError> {
    let current = authority.project_root();
    let resolve =
        |path: &Path| adocweave_project::absolute_path(current, path).map_err(CliError::Project);
    let citation_file = |path: &Option<PathBuf>, name: &str| {
        path.as_deref()
            .ok_or_else(|| {
                CliError::Usage(format!("visible slide citations require --{name} FILE"))
            })
            .and_then(resolve)
    };
    let bibliography = citations
        .then(|| citation_file(&options.bibliography, "bibliography"))
        .transpose()?;
    let style = citations
        .then(|| citation_file(&options.csl_style, "csl-style"))
        .transpose()?;
    let locale = citations
        .then(|| citation_file(&options.csl_locale, "csl-locale"))
        .transpose()?;
    let macro_file = if math {
        options.math_macros.as_deref().map(resolve).transpose()?
    } else {
        None
    };
    let paths = bibliography
        .iter()
        .chain(&style)
        .chain(&locale)
        .chain(&macro_file)
        .cloned()
        .collect::<Vec<_>>();
    let resources = authority
        .read_binary_resources(roots, &paths, limits, cancellation)
        .map_err(CliError::Project)?;
    let bytes = resources
        .iter()
        .map(|resource| resource.bytes.len() as u64)
        .sum::<u64>();
    let by_path = resources
        .iter()
        .map(|resource| (&resource.path, resource))
        .collect::<BTreeMap<_, _>>();
    let macros = macro_file
        .as_ref()
        .map(|path| json(by_path[path], "math macro"))
        .transpose()?;
    let csl = if let (Some(bibliography), Some(style), Some(locale)) = (bibliography, style, locale)
    {
        Some(Csl {
            items: json(by_path[&bibliography], "bibliography")?,
            style: xml(by_path[&style], "style")?,
            locale: xml(by_path[&locale], "locale")?,
        })
    } else {
        None
    };
    Ok(DataInputs {
        macros,
        csl,
        remaining: ProjectResourceLimits {
            max_files: limits.max_files.saturating_sub(resources.len()),
            max_total_bytes: limits.max_total_bytes.saturating_sub(bytes),
            ..limits
        },
        resources,
    })
}
