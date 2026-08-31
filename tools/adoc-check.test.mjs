import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import test from "node:test";
import {
  trackedAdocPaths,
  trackedAdocPlan,
  validateCurrentDocumentAdrLinks,
} from "./adoc-check.mjs";

test("追跡対象を一度ずつ検査し、規範文書だけをlocal target検査へ渡す", () => {
  const plan = trackedAdocPlan([
    "docs/空白 を含む文書.adoc",
    "fixtures/case with spaces.adoc",
    "editors/vscode/test/fixtures/workspace/root.adoc",
    "fuzz/corpus/analyze/basic.adoc",
  ]);
  assert.deepEqual(plan, [
    { path: "docs/空白 を含む文書.adoc", failOn: "warning", localTargets: true },
    {
      path: "editors/vscode/test/fixtures/workspace/root.adoc",
      failOn: "error",
      localTargets: false,
    },
    { path: "fixtures/case with spaces.adoc", failOn: "error", localTargets: false },
    { path: "fuzz/corpus/analyze/basic.adoc", failOn: "error", localTargets: false },
  ]);
});

test("重複した列挙とAsciiDoc以外のpathを拒否する", () => {
  assert.throws(
    () => trackedAdocPlan(["docs/README.adoc", "docs/README.adoc"]),
    /重複/,
  );
  assert.throws(() => trackedAdocPlan(["docs/README.md"]), /AsciiDoc以外/);
});

test("GitのNUL区切り出力から空白を含むpathを失わず取得する", () => {
  const fakeGit = (command, args, options) => {
    assert.deepEqual([command, args, options], [
      "git",
      ["ls-files", "-z", "--", "*.adoc"],
      { encoding: null },
    ]);
    return {
      status: 0,
      stdout: Buffer.from("README.adoc\0docs/空白 を含む文書.adoc\0"),
      stderr: Buffer.alloc(0),
    };
  };
  assert.deepEqual(trackedAdocPaths(fakeGit), [
    "README.adoc",
    "docs/空白 を含む文書.adoc",
  ]);
});

test("repositoryの全追跡AsciiDocを動的な検査計画へ含める", () => {
  const tracked = execFileSync("git", ["ls-files", "-z", "--", "*.adoc"]);
  const paths = tracked.toString("utf8").split("\0").filter(Boolean);
  const plan = trackedAdocPlan(paths);
  assert.equal(plan.length, paths.length);
  assert.deepEqual(new Set(plan.map((entry) => entry.path)), new Set(paths));
  for (const path of [
    "docs/developer-guide/vscode-development.adoc",
    "docs/developer-guide/adr/0013-vscode-release-boundary.adoc",
  ]) {
    assert.equal(plan.find((entry) => entry.path === path)?.localTargets, true, path);
  }
});

test("現行文書から置換済みADRへの参照を拒否する", () => {
  const sources = {
    "CONTRIBUTING.adoc": "xref:docs/developer-guide/adr/0001-old.adoc[古い判断]",
    "docs/developer-guide/adr/0001-old.adoc": ":status: 置換済み。ADR 0002により置換\n",
    "docs/developer-guide/adr/0002-current.adoc": ":status: 採用\n",
  };
  assert.throws(
    () => validateCurrentDocumentAdrLinks(sources),
    /CONTRIBUTING\.adoc -> docs\/developer-guide\/adr\/0001-old\.adoc/,
  );
});

test("現行ADRとADR内の判断履歴への参照を許可する", () => {
  assert.doesNotThrow(() => validateCurrentDocumentAdrLinks({
    "guide.adoc": "xref:docs/developer-guide/adr/0002-current.adoc#decision[現在の判断]",
    "docs/developer-guide/adr/0001-old.adoc":
      ":status: 置換済み。ADR 0002により置換\n\nxref:0002-current.adoc[置換先]",
    "docs/developer-guide/adr/0002-current.adoc":
      ":status: 採用\n\nxref:0001-old.adoc[判断履歴]",
    "docs/developer-guide/adr/index.adoc": "xref:0001-old.adoc[ADR 0001]",
  }));
});
