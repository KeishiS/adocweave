import { readFileSync } from "node:fs";

/** The fields of the extension `package.json` these scripts read. */
export interface ExtensionManifest {
  name: string;
  publisher: string;
  version: string;
  main?: string;
  private?: boolean;
  homepage?: string;
  repository?: { type?: string; url?: string };
  scripts?: Record<string, string>;
  publishConfig?: unknown;
  contributes?: { languages?: Array<{ id?: string }> };
}

/** The fields of the repository `release-manifest.json` these scripts read. */
export interface ReleaseManifest {
  packageVersion: string;
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
