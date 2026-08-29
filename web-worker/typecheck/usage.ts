import {
  AdocWeaveClient,
  AdocWeaveError,
  type AnalyzeRequest,
  PROTOCOL_SCHEMA_VERSION,
  defaultAssetUrls,
} from "../index.mjs";

const client = new AdocWeaveClient({
  ...defaultAssetUrls(new URL("../worker/index.mjs", import.meta.url)),
});

async function render(source: string) {
  try {
    const result = await client.analyze({
      source: { text: source, id: "editor.adoc" },
      products: {
        html: {
          activeUrls: { allowedSchemes: ["http", "https"] },
          documentMode: "fragment",
        },
        diagnostics: true,
        document: true,
      },
    });
    console.log(result.html, result.document?.formulas[0]?.source, result.diagnostics);
  } catch (error) {
    if (error instanceof AdocWeaveError) console.error(error.code);
  }
}

void render("= Typed");
const request: AnalyzeRequest = {
  source: { text: "include::part.adoc[]" },
  products: { html: true },
  resources: { documents: { "part.adoc": "Text" } },
};
void client.analyze(request, { signal: new AbortController().signal });
const renderDefaultsRequest: AnalyzeRequest = {
  source: { text: "Text" },
  products: { html: true },
  resources: {
    references: [{
      sourceStart: 0,
      sourceEnd: 1,
      outcome: { status: "resolved", href: "https://example.test" },
    }],
    assets: [{
      sourceStart: 1,
      sourceEnd: 2,
      outcome: {
        status: "resolved",
        href: "https://example.test/a",
        mediaType: "text/plain",
        byteLength: 42,
      },
    }],
    citations: [{ sourceStart: 2, sourceEnd: 3, outcome: { status: "resolved" } }],
    bibliography: { title: "References" },
  },
};
void client.analyze(renderDefaultsRequest);
console.log(PROTOCOL_SCHEMA_VERSION);
client.dispose();
