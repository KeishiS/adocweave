pub(crate) fn render_analysis(analysis: &adocweave_core::Analysis) -> String {
    adocweave_core::semantic::render_symbols_json(&adocweave_core::semantic::document_symbols(
        analysis.document(),
    ))
}
