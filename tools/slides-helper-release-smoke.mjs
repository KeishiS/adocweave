import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { assertSignatureAudit } from "./wasm-npm-smoke.mjs";
import { validateSlidesRuntimeDependencies, verifyInstalledNotices } from "./verify-slides-dependencies.mjs";

const helper = fileURLToPath(new URL("../packages/slides-helper/", import.meta.url));
const manifest = JSON.parse(await readFile(join(helper, "package.json"), "utf8"));
const lock = JSON.parse(await readFile(join(helper, "package-lock.json"), "utf8"));
const fixture = JSON.parse(await readFile(join(helper, "fixtures/research.json"), "utf8"));
const evidence = JSON.parse(await readFile(join(helper, "licenses/evidence.json"), "utf8"));

// Use the installed archive rather than the checkout's modules or npm links.
export async function verifyInstalledHelper(archive, { published = false } = {}) {
  const temporary = await mkdtemp(join(tmpdir(), "adocweave-slides-install-"));
  try {
    const cache = join(temporary, ".npm-cache");
    const install = ["install", "--ignore-scripts", "--no-audit", "--no-fund", "--prefix", temporary, "--cache", cache];
    install.push("--registry=https://registry.npmjs.org");
    if (!published) install.push("--offline");
    install.push(published ? `${manifest.name}@${manifest.version}` : resolve(archive));
    execFileSync("npm", install, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    const installed = join(temporary, "node_modules", "@adocweave", "slides-helper");
    const tree = JSON.parse(execFileSync("npm", ["ls", "--all", "--json", "--prefix", temporary], { encoding: "utf8" }));
    const versions = new Map();
    const visit = (dependencies = {}) => {
      for (const [name, item] of Object.entries(dependencies)) {
        if (name !== manifest.name) {
          if (!versions.has(name)) versions.set(name, new Set());
          versions.get(name).add(item.version);
        }
        visit(item.dependencies);
      }
    };
    visit(tree.dependencies);
    const expected = new Map(Object.entries(lock.packages).filter(([path]) => path)
      .map(([path, item]) => [path.slice(path.lastIndexOf("node_modules/") + 13), new Set([item.version])]));
    assert.deepEqual(versions, expected, "npm installation must use the complete audited runtime tree.");
    verifyInstalledNotices(installed, validateSlidesRuntimeDependencies(manifest, lock, evidence), evidence);
    const run = (request) => JSON.parse(execFileSync(process.execPath, [join(installed, "bin.mjs")], {
      input: JSON.stringify(request), encoding: "utf8", timeout: 30_000, maxBuffer: 32 * 1024 * 1024,
    }));
    const empty = run({ schemaVersion: 1, eqnums: "none", scopes: {
      body: { equations: [], citations: [] }, notes: { equations: [], citations: [] },
    } });
    assert.deepEqual(empty.diagnostics, []);
    assert.deepEqual(empty.notices, { math: null, citations: null });
    const result = run(fixture);
    assert.deepEqual(result.diagnostics, []);
    for (const scope of ["body", "notes"]) {
      for (const kind of ["equations", "citations"]) {
        assert.deepEqual(result.scopes[scope][kind].map(({ key, status }) => ({ key, status })),
          fixture.scopes[scope][kind].map(({ key }) => ({ key, status: "ok" })));
      }
      assert.ok(result.scopes[scope].equations.every((equation) => equation.svg.includes("<svg")
        && Object.keys(equation).sort().join(",") === "key,status,svg"));
      assert.equal(result.scopes[scope].bibliography.length, fixture.scopes[scope].citations.length);
    }
    assert.match(result.notices.math.fontAttribution, /Tsolomitis/);
    assert.match(result.notices.citations.license, /Exhibit B/);
    if (published) {
      const report = JSON.parse(execFileSync("npm", ["audit", "signatures", "--json", "--include-attestations", "--prefix", temporary, "--cache", cache], {
        encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
      }));
      assertSignatureAudit(report, manifest);
    }
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

async function main() {
  if (process.argv[2] === "--published") {
    await verifyInstalledHelper(undefined, { published: true });
  } else if (process.argv[2] === "--pack") {
    const destination = resolve("target/distrib");
    await mkdir(destination, { recursive: true });
    const [packed] = JSON.parse(execFileSync("npm", ["pack", "--ignore-scripts", "--json", "--pack-destination", destination], {
      cwd: helper, encoding: "utf8",
    }));
    await verifyInstalledHelper(join(destination, packed.filename));
    process.stdout.write(`Verified ${packed.filename}\n`);
  } else {
    throw new Error("Expected --pack or --published.");
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => { process.stderr.write(`${error.stack ?? error}\n`); process.exitCode = 1; });
}
