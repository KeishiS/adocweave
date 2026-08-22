import { readFileSync } from "node:fs";

/** The fields of the extension `package.json` these scripts read. */
export interface ExtensionManifest {
  capabilities?: { untrustedWorkspaces?: { description?: string; supported?: boolean } };
  name: string;
  publisher: string;
  version: string;
  main?: string;
  private?: boolean;
  homepage?: string;
  repository?: { type?: string; url?: string };
  scripts?: Record<string, string>;
  publishConfig?: unknown;
  contributes?: {
    commands?: Array<{ command?: string }>;
    configuration?: { properties?: Record<string, { scope?: string }> };
    languages?: Array<{ id?: string }>;
  };
}

/**
 * Parses a JSON file as the given shape.
 *
 * The shape is asserted, not validated: callers check the fields they depend
 * on, and the VSIX gates compare the values against the release manifest.
 */
export function readJson<T>(path: string | URL): T {
  return JSON.parse(readFileSync(path, "utf8")) as T;
}
