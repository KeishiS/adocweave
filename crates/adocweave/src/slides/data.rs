//! Local helper input files and bundled CSL defaults.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adocweave_core::CancellationCheck;
use adocweave_project::{ProjectAuthority, ProjectBinaryResource, ProjectResourceLimits};

use super::helper::{Csl, Macro};
use crate::arguments::SlidesData;
use crate::cli_error::CliError;

const DEFAULT_CSL_STYLE: &str = include_str!("../../assets/slides/default.csl");
const DEFAULT_CSL_LOCALE: &str = include_str!("../../assets/slides/locale-en-US.xml");

pub(super) struct DataInputs {
    pub(super) macros: Option<Vec<Macro>>,
    pub(super) csl: Option<Csl>,
    pub(super) resources: Vec<ProjectBinaryResource>,
    pub(super) remaining: ProjectResourceLimits,
}

fn resource_error(resource: &ProjectBinaryResource, message: String) -> CliError {
    CliError::SlidesResources {
        message,
        observations: vec![resource.observation()],
    }
}

fn json<T: serde::de::DeserializeOwned>(
    resource: &ProjectBinaryResource,
    purpose: &str,
) -> Result<T, CliError> {
    serde_json::from_slice(&resource.bytes).map_err(|error| {
        resource_error(
            resource,
            format!(
                "{}:{}:{}: invalid {purpose} JSON: {error}",
                resource.path.display(),
                error.line(),
                error.column()
            ),
        )
    })
}

fn xml(resource: &ProjectBinaryResource, element: &str) -> Result<String, CliError> {
    let text = std::str::from_utf8(&resource.bytes).map_err(|_| {
        resource_error(
            resource,
            format!("{}: {element} XML must be UTF-8", resource.path.display()),
        )
    })?;
    if text.len() > 512 * 1024 || text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err(resource_error(
            resource,
            format!(
                "{}: {element} XML exceeds limits or contains an unsupported document type",
                resource.path.display()
            ),
        ));
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
        resource_error(
            resource,
            format!(
                "{}:{}:{}: invalid {element} XML: {error}",
                resource.path.display(),
                error.pos().row,
                error.pos().col
            ),
        )
    })?;
    let root = document.root_element();
    if root.tag_name().name() != element
        || root.tag_name().namespace() != Some("http://purl.org/net/xbiblio/csl")
        || root.descendants().any(|node| node.ancestors().count() > 64)
    {
        return Err(resource_error(
            resource,
            format!(
                "{}: expected a bounded CSL {element} XML document",
                resource.path.display()
            ),
        ));
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
    let bibliography = if citations {
        let path = options.bibliography.as_deref().ok_or_else(|| {
            CliError::Usage("visible slide citations require --bibliography FILE".to_owned())
        })?;
        Some(resolve(path)?)
    } else {
        None
    };
    let style = if citations {
        options.csl_style.as_deref().map(resolve).transpose()?
    } else {
        None
    };
    let locale = if citations {
        options.csl_locale.as_deref().map(resolve).transpose()?
    } else {
        None
    };
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
    let csl = if let Some(bibliography) = bibliography {
        Some(Csl {
            items: json(by_path[&bibliography], "bibliography")?,
            style: match style {
                Some(path) => xml(by_path[&path], "style")?,
                None => DEFAULT_CSL_STYLE.to_owned(),
            },
            locale: match locale {
                Some(path) => xml(by_path[&path], "locale")?,
                None => DEFAULT_CSL_LOCALE.to_owned(),
            },
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
