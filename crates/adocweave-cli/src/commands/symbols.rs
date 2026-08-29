pub(crate) fn render_analysis(analysis: &adocweave::Analysis) -> String {
    adocweave::semantic::render_symbols_json(&adocweave::semantic::document_symbols(
        analysis.document(),
    ))
}
