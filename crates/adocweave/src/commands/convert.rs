use adocweave_core::output::html::RenderPolicy;

use super::html_policy;

#[derive(Debug)]
pub(crate) enum Error {
    Html(html_policy::Error),
}

pub(crate) fn render_analysis(
    analysis: &adocweave_core::Analysis,
    render_policy: &RenderPolicy,
) -> Result<String, Error> {
    Ok(
        html_policy::render_checked(analysis.document(), render_policy)
            .map_err(Error::Html)?
            .html,
    )
}
