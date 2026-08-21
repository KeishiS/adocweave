import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const tsc = fileURLToPath(new URL("../node_modules/typescript/bin/tsc", import.meta.url));
execFileSync(process.execPath, [tsc, "-p", "tsconfig.test.json"], { stdio: "inherit" });

// The compiled extension host tests read these fixtures and resources next to
// the emitted JavaScript, so copy them into the same layout.
const copies: ReadonlyArray<readonly [source: string, target: string]> = [
  ["resources/platforms.json", "dist-test/resources/platforms.json"],
  ["syntaxes/asciidoc.tmLanguage.json", "dist-test/syntaxes/asciidoc.tmLanguage.json"],
  ["test/fixtures/grammar-scopes.json", "dist-test/test/fixtures/grammar-scopes.json"],
];
for (const [source, target] of copies) {
  mkdirSync(target.slice(0, target.lastIndexOf("/")), { recursive: true });
  copyFileSync(source, target);
}
