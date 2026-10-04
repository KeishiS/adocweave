//! Local author CSS, copied after the fixed theme without discovering its assets.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use adocweave_core::CancellationCheck;
use adocweave_core::output::html::StylesheetSource;
use adocweave_project::{
    BundleFile, BundleMediaType, ProjectAuthority, ProjectBinaryResource, ProjectConfig,
    ProjectResourceLimits,
};
use sha2::{Digest as _, Sha256};

use crate::cli_error::CliError;

pub(super) struct Styles {
    pub(super) files: Vec<BundleFile>,
    pub(super) links: Vec<String>,
    pub(super) resources: Vec<ProjectBinaryResource>,
    pub(super) remaining: ProjectResourceLimits,
}

pub(super) fn load(
    config: &ProjectConfig,
    authority: &ProjectAuthority,
    roots: &[PathBuf],
    limits: ProjectResourceLimits,
    cancellation: &dyn CancellationCheck,
) -> Result<Styles, CliError> {
    let policy = &config.html_policy().stylesheets;
    if !config.stylesheet_urls().is_empty()
        || policy
            .sources
            .iter()
            .any(|source| matches!(source, StylesheetSource::External(_)))
    {
        return Err(CliError::Usage(
            "revealjs output requires local CSS; stylesheet URLs are unsupported".to_owned(),
        ));
    }
    let count = config
        .stylesheet_files()
        .len()
        .saturating_add(policy.sources.len());
    if count > policy.max_sources as usize {
        return Err(CliError::Stylesheet(format!(
            "stylesheet count exceeds the limit of {}",
            policy.max_sources
        )));
    }
    let resources = authority
        .read_binary_resources(roots, config.stylesheet_files(), limits, cancellation)
        .map_err(CliError::Project)?;
    let mut sources = Vec::new();
    let by_path = resources
        .iter()
        .map(|resource| (&resource.path, resource))
        .collect::<BTreeMap<_, _>>();
    for path in config.stylesheet_files() {
        let resource = by_path[path];
        std::str::from_utf8(&resource.bytes).map_err(|_| {
            CliError::Stylesheet(format!(
                "stylesheet {} must be UTF-8",
                resource.path.display()
            ))
        })?;
        sources.push(resource.bytes.clone());
    }
    for source in &policy.sources {
        if let StylesheetSource::Inline(text) = source {
            sources.push(text.as_bytes().to_vec());
        }
    }
    let mut files = Vec::new();
    let mut links = Vec::new();
    let mut seen = BTreeSet::new();
    for bytes in sources {
        if bytes.len() > policy.max_inline_bytes as usize {
            return Err(CliError::Stylesheet(format!(
                "stylesheet exceeds the limit of {} bytes",
                policy.max_inline_bytes
            )));
        }
        let hash = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = format!("assets/{hash}.css");
        if seen.insert(path.clone()) {
            links.push(path.clone());
            files.push(BundleFile {
                path,
                media_type: BundleMediaType::Css,
                bytes,
            });
        }
    }
    // Inline CSS is already bounded output configuration; file reads consume
    // the request's shared resource budget and retain their acquired observations.
    let bytes = resources
        .iter()
        .map(|resource| resource.bytes.len() as u64)
        .sum::<u64>();
    Ok(Styles {
        files,
        links,
        remaining: ProjectResourceLimits {
            max_files: limits.max_files.saturating_sub(resources.len()),
            max_total_bytes: limits.max_total_bytes.saturating_sub(bytes),
            ..limits
        },
        resources,
    })
}
