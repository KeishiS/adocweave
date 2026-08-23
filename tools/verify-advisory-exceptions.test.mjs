import assert from "node:assert/strict";
import test from "node:test";

import { validateAdvisoryExceptions } from "./verify-advisory-exceptions.mjs";

const exception = {
  id: "RUSTSEC-2026-0001",
  reason: "理由: 影響するAPIを呼び出さないため; 期限: 2099-12-31; Issue: https://github.com/KeishiS/adocweave/issues/999",
};

test("空のadvisory例外を受理する", () => {
  validateAdvisoryExceptions({}, "2026-08-23");
});

test("advisory例外に理由、将来の期限および追跡Issueを求める", () => {
  validateAdvisoryExceptions({ advisories: { ignore: [exception] } }, "2026-08-23");
  for (const changed of [
    { ...exception, reason: "期限とIssueがありません" },
    { ...exception, reason: exception.reason.replace("2099-12-31", "2026-08-23") },
    { ...exception, reason: exception.reason.replace("KeishiS/adocweave", "example/other") },
  ]) {
    assert.throws(
      () => validateAdvisoryExceptions({ advisories: { ignore: [changed] } }, "2026-08-23"),
    );
  }
});
