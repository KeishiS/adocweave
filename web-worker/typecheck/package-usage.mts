import {
  AdocWeaveClient,
  type AdocWeaveResult,
  defaultAssetUrls,
} from "@adocweave/browser";

const client = new AdocWeaveClient(
  defaultAssetUrls(new URL("./node_modules/@adocweave/browser/worker/index.mjs", import.meta.url)),
);
const result: Promise<AdocWeaveResult> = client.analyze({ source: "= Package import" });
console.log((await result).html);
client.dispose();
