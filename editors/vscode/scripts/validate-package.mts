import { type ExtensionManifest, type ReleaseManifest, readJson } from "./manifests.mts";

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
const release = readJson<ReleaseManifest>("../../release-manifest.json");
const language = readJson<LanguageConfiguration>("language-configuration.json");
const grammar = readJson<Grammar>("syntaxes/asciidoc.tmLanguage.json");

if (packageJson.version !== release.packageVersion) {
  throw new Error("The VS Code extension version does not match the release manifest");
}
if (packageJson.private !== true || packageJson.scripts?.publish || packageJson.publishConfig) {
  throw new Error("The VS Code extension must keep registry publishing disabled");
}
if (
  packageJson.homepage !== "https://github.com/KeishiS/adocweave" ||
  packageJson.repository?.url !== "https://github.com/KeishiS/adocweave.git"
) {
  throw new Error("The VS Code extension repository URL does not match the canonical name");
}
if (
  packageJson.main !== "./dist/extension.cjs" ||
  packageJson.contributes?.languages?.[0]?.id !== "asciidoc"
) {
  throw new Error("The VS Code extension entry point or language contribution is invalid");
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
