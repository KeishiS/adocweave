import { readFileSync } from "node:fs";

/** The fields of the extension `package.json` these scripts read. */
export interface ExtensionManifest {
  capabilities?: { untrustedWorkspaces?: { description?: string; supported?: boolean } };
  displayName: string;
  devDependencies?: Record<string, string>;
  engines?: { vscode?: string };
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
 * The shape is asserted, not validated: callers check the fields they depend on.
 */
export function readJson<T>(path: string | URL): T {
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

/**
 * Returns the lowest Visual Studio Code version the extension declares support for.
 *
 * 検査はこの版で行います。宣言した下限を検査しないと、実際には動かない版を
 * 対応範囲として公開してしまいます。0.47.0の`^1.125.0`と0.47.1の`^1.91.0`は
 * どちらもその失敗でした。
 */
export function supportedVSCodeFloor(manifest: ExtensionManifest): string {
  const range = manifest.engines?.vscode;
  const floor = /^\^(\d+\.\d+\.\d+)$/.exec(range ?? "")?.[1];
  if (!floor) {
    throw new Error(`engines.vscodeは^MAJOR.MINOR.PATCH形式が必要です：${range}`);
  }
  return floor;
}

/** Rejects API types that require a newer editor than the declared minimum. */
export function validateVSCodeTypesCompatibility(manifest: ExtensionManifest): void {
  const floor = supportedVSCodeFloor(manifest);
  const types = manifest.devDependencies?.["@types/vscode"];
  if (typeof types !== "string" || !/^\d+\.\d+\.\d+$/.test(types)) {
    throw new Error("@types/vscode must be pinned to a MAJOR.MINOR.PATCH version");
  }
  // DefinitelyTyped patch versions can change without changing the VS Code API.
  const [engineMajor = 0, engineMinor = 0] = floor.split(".").map(Number);
  const [typesMajor = 0, typesMinor = 0] = types.split(".").map(Number);
  if (typesMajor > engineMajor || (typesMajor === engineMajor && typesMinor > engineMinor)) {
    throw new Error(`@types/vscode ${types} requires a newer editor than engines.vscode ${floor}`);
  }
}
