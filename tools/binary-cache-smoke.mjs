import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import {
  createNativeSmokeDeadline,
  smokeLsp,
} from "./native-lsp-smoke.mjs";
import { workspaceVersion } from "./release-version.mjs";

export async function verifyBinaryCachePackage(
  binary,
  expectedVersion = workspaceVersion(),
  {
    createDeadline = createNativeSmokeDeadline,
    execute = execFileSync,
    runLsp = smokeLsp,
  } = {},
) {
  const reported = JSON.parse(execute(binary, ["--version", "--json"], { encoding: "utf8" }));
  if (reported.packageVersion !== expectedVersion) {
    throw new Error("binary cache package version does not match the workspace version");
  }

  const deadline = createDeadline();
  try {
    await runLsp(binary, ["lsp"], expectedVersion, deadline);
  } finally {
    deadline.dispose();
  }
}

async function main() {
  const [binary, ...unexpected] = process.argv.slice(2);
  if (!binary || unexpected.length > 0) {
    throw new Error("usage: node tools/binary-cache-smoke.mjs ADOCWEAVE_BINARY");
  }
  await verifyBinaryCachePackage(binary);
  process.stdout.write("binary cache package smoke passed\n");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
