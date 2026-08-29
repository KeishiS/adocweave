import { existsSync } from "node:fs";
import process from "node:process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const [mode] = process.argv.slice(2);
if (!["installation", "smoke"].includes(mode) || process.argv.length !== 3) {
  process.stderr.write("usage: node tools/local-native-check.mjs installation|smoke\n");
  process.exit(2);
}

const targetByHost = new Map([
  ["darwin/arm64", "aarch64-apple-darwin"],
  ["linux/arm64", "aarch64-unknown-linux-musl"],
  ["linux/x64", "x86_64-unknown-linux-musl"],
  ["win32/x64", "x86_64-pc-windows-msvc"],
]);
const target = targetByHost.get(`${process.platform}/${process.arch}`);
if (!target) {
  throw new Error(`local native checks do not support ${process.platform}/${process.arch}`);
}

const candidate = resolve(process.env.NATIVE_ARTIFACT_DIR ?? "target/distrib");
if (!existsSync(candidate)) {
  throw new Error(`native artifact directory does not exist: ${candidate}`);
}

const script = mode === "smoke" ? "native-release-smoke.mjs" : "release-installation-e2e.mjs";
const args = [fileURLToPath(new URL(script, import.meta.url)), candidate, target];
if (mode === "installation" && process.env.NATIVE_MANIFEST) {
  args.push(resolve(process.env.NATIVE_MANIFEST));
}
execFileSync(process.execPath, args, { stdio: "inherit" });
