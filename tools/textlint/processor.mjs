import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

import { createParseText } from "../../packages/textlint-plugin-asciidoc/bridge.mjs";
import { createProcessorClass } from "../../packages/textlint-plugin-asciidoc/processor.mjs";

const repositoryRoot = fileURLToPath(new URL("../../", import.meta.url));
const require = createRequire(import.meta.url);

let bridge;
function loadBridge() {
  bridge ??= require(
    `${repositoryRoot}target/adocweave-textlint-node/adocweave_textlint_wasm.js`
  );
  return bridge;
}

const parseText = createParseText({
  bridgeLoader: loadBridge
});

// リポジトリ内の検査でも、配布するProcessorとTxtAST adapterをそのまま使います。
// WebAssemblyだけはpackage作成前の専用build成果物へ接続します。
export const Processor = createProcessorClass(parseText);

export default { Processor };
