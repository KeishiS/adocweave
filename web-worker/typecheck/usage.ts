import {
  AdocWeaveClient,
  AdocWeaveClientError,
  AdocWeaveResult,
  AnalyzeRequest,
  PROTOCOL_SCHEMA_VERSION,
  defaultAssetUrls,
} from "../index.mjs";

const client = new AdocWeaveClient({
  ...defaultAssetUrls(new URL("../worker/index.mjs", import.meta.url)),
});
let sourceRevision = 0;
let debounce: ReturnType<typeof setTimeout> | undefined;
let active: AbortController | undefined;

function schedule(source: string) {
  const revision = ++sourceRevision;
  active?.abort();
  active = new AbortController();
  const cancellation = active;
  clearTimeout(debounce);
  debounce = setTimeout(async () => {
    try {
      const result: AdocWeaveResult = await client.analyze(
        { source },
        { signal: cancellation.signal },
      );
      if (revision !== sourceRevision) return;
      const formulaSource: string | undefined = result.projection?.formulas[0]?.source;
      console.log(result.html, formulaSource);
    } catch (error) {
      if (error instanceof AdocWeaveClientError) console.error(error.code);
      else if (error instanceof DOMException && error.name === "AbortError") console.error(error.message);
    }
  }, 40);
}

schedule("= Promise");
const protocolSchemaVersion: number = PROTOCOL_SCHEMA_VERSION;
console.log(protocolSchemaVersion);
const next = client.analyze({
  source: "= Typed",
  analysisOptions: {
    syntax: {
      limits: { maxInputBytes: 1024 * 1024 },
    },
  },
  renderPolicy: {
    activeUrls: { allowResolvedRootRelative: true },
    externalLinks: { openInNewContext: true, noreferrer: true },
    sourceLanguages: { allowed: ["rust"], unknown: "diagnostic" },
    mathLanguages: ["latex"],
    unresolvedReferences: "label-only",
    resources: { images: false, media: false },
    documentMode: "complete",
    stylesheets: [
      { kind: "inline", css: "p { margin: 0; }" },
      { kind: "external", url: "https://example.com/theme.css" },
    ],
  },
}, { signal: new AbortController().signal });
console.log((await next).html);
const invalidProducts: AnalyzeRequest = {
  source: "invalid product override",
  // @ts-expect-error productsを指定する場合は全flagが必要です。
  products: { html: true },
};
console.log(invalidProducts.source);
client.dispose();
