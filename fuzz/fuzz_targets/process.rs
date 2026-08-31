#![no_main]

use adocweave_core::{AnalysisOptions, Engine};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Ok(source) = std::str::from_utf8(input) else {
        return;
    };
    if let Ok(analysis) = Engine::new(AnalysisOptions::default()).analyze(source) {
        let _ = adocweave_core::output::html::render(
            analysis.document(),
            &adocweave_core::output::html::RenderPolicy::default(),
        );
        let _ = adocweave_core::output::formatter::format_analysis(
            &analysis,
            &adocweave_core::output::formatter::FormatConfig::default(),
            &adocweave_core::NeverCancel,
        );
        let _ = analysis.document().symbols();
        let _ = analysis.diagnostics();
    }
});
