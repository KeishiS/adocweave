import {
  AdocWeaveClient,
  type AnalyzeResult,
  defaultAssetUrls,
} from "@adocweave/wasm";

const client = new AdocWeaveClient(
  defaultAssetUrls(new URL("./node_modules/@adocweave/wasm/worker/index.mjs", import.meta.url)),
);
const result: Promise<AnalyzeResult> = client.analyze({
  source: { text: "= Package import" },
  products: { html: true },
});
console.log((await result).html);
client.dispose();
