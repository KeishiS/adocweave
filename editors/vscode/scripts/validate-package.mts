import { readFileSync } from "node:fs";

import { type ExtensionManifest, readJson } from "./manifests.mts";

// The package manifest is the extension version authority. Validate the identity,
// changelog, entry point, language contribution, and TextMate grammar together.
// A broken grammar pattern silently stops highlighting rather than failing anything else.

interface LanguageConfiguration {
  brackets?: unknown;
}

interface GrammarPattern {
  begin?: string;
  end?: string;
  match?: string;
}

interface Grammar {
  scopeName?: string;
  repository?: Record<string, { patterns?: GrammarPattern[] }>;
}

const packageJson = readJson<ExtensionManifest>("package.json");
const changelog = readFileSync("CHANGELOG.md", "utf8");
const language = readJson<LanguageConfiguration>("language-configuration.json");
const grammar = readJson<Grammar>("syntaxes/asciidoc.tmLanguage.json");

if (
  packageJson.name !== "adocweave" ||
  packageJson.displayName !== "AdocWeave Language Support" ||
  packageJson.publisher !== "adocweave" ||
  packageJson.private !== true ||
  !/^0\.\d+\.\d+$/u.test(packageJson.version) ||
  !changelog.includes(`## [${packageJson.version}]`) ||
  packageJson.main !== "./dist/extension.cjs" ||
  packageJson.contributes?.languages?.[0]?.id !== "asciidoc"
) {
  throw new Error("The VS Code extension entry point or language contribution is invalid");
}
const serverSettings = packageJson.contributes?.configuration?.properties;
if (
  packageJson.capabilities?.untrustedWorkspaces?.supported !== false ||
  !packageJson.capabilities.untrustedWorkspaces.description?.trim() ||
  serverSettings?.["adocweave.server.path"]?.scope !== "machine" ||
  "adocweave.server.download" in (serverSettings ?? {}) ||
  packageJson.contributes?.commands?.some(
    ({ command }) => command === "adocweave.clearManagedServer",
  )
) {
  throw new Error("The Language Server trust boundary or settings are invalid");
}
if (!Array.isArray(language.brackets) || grammar.scopeName !== "text.asciidoc") {
  throw new Error("The AsciiDoc language configuration or TextMate grammar is invalid");
}

// Every grammar regular expression must compile; `\1` back references are
// replaced with a representative delimiter because JavaScript rejects them
// outside a capture.
const patternFields = ["begin", "end", "match"] as const;
for (const repository of Object.values(grammar.repository ?? {})) {
  for (const pattern of repository.patterns ?? []) {
    for (const field of patternFields) {
      const source = pattern[field];
      if (source) new RegExp(source.replaceAll("\\1", "----"), "u");
    }
  }
}

process.stdout.write("Validated the VS Code extension manifest and grammar.\n");
