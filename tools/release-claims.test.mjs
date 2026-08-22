import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { changedContracts, contractShape } from "./release-claims.mjs";
import { PUBLIC_PROTOCOL_SCHEMA_VERSION } from "./release-policy.mjs";
import { PROTOCOL_SCHEMA_VERSION as WORKER_PROTOCOL_SCHEMA_VERSION } from "../web-worker/worker-protocol.mjs";
import {
  CONTRACT_SOURCES,
  CONTRACT_VERSION_FIELDS,
  UNCHANGED_CONTRACTS,
} from "./release-notes.mjs";

test("versionだけが違う契約は変更として報告しない", () => {
  // packageVersionはreleaseごとに必ず変わります。これを差分として数えると、
  // 形を変えていない契約が毎回「変更あり」になり、検査が役に立ちません。
  const before = JSON.stringify({ packageVersion: "1.0.0", schemaVersion: 9, fields: ["a"] });
  const after = JSON.stringify({ packageVersion: "1.0.1", schemaVersion: 9, fields: ["a"] });
  assert.equal(contractShape(before, "config/adocweave.schema.json"), contractShape(after, "config/adocweave.schema.json"));
});

test("形が違う契約は変更として報告する", () => {
  const before = JSON.stringify({ packageVersion: "1.0.0", schemaVersion: 9 });
  const after = JSON.stringify({ packageVersion: "1.0.1", schemaVersion: 10 });
  assert.notEqual(
    contractShape(before, "config/adocweave.schema.json"),
    contractShape(after, "config/adocweave.schema.json"),
  );
});

test("変更のあった契約だけを名前で返す", () => {
  const contents = {
    "v1:a.json": JSON.stringify({ packageVersion: "1.0.0", shape: 1 }),
    "HEAD:a.json": JSON.stringify({ packageVersion: "1.0.1", shape: 1 }),
    "v1:b.json": JSON.stringify({ packageVersion: "1.0.0", shape: 1 }),
    "HEAD:b.json": JSON.stringify({ packageVersion: "1.0.1", shape: 2 }),
  };
  const read = (revision, path) => contents[`${revision}:${path}`];
  assert.deepEqual(
    changedContracts("v1", { 変わらない: "a.json", 変わった: "b.json" }, read),
    ["変わった"],
  );
});

test("宣言した契約のうち正本を持つものは検査対象になる", () => {
  // 正本の対応を静かに外せば検査は素通りします。対応表が空になっていないこと、
  // および対応表の値が実在するpathであることを固定します。
  assert.notEqual(Object.keys(CONTRACT_SOURCES).length, 0);
  for (const [name, path] of Object.entries(CONTRACT_SOURCES)) {
    assert.match(path, /^[a-z]/, name);
    // 契約の正本はJSONか、Rustから生成したTypeScript宣言です。
    assert.match(path, /\.(json|d\.mts)$/, name);
  }
  assert.ok(
    UNCHANGED_CONTRACTS.some((name) => name in CONTRACT_SOURCES),
    "変更していないと述べる契約のうち、一つも検査できていません",
  );
});

test("versionを表すfieldを明示している", () => {
  assert.deepEqual(CONTRACT_VERSION_FIELDS, ["packageVersion"]);
});

test("WASM requestのversion fieldとprotocol schemaの識別子を区別する", () => {
  // requestはpackageVersionだけを持ち、protocol schemaの識別子はrequestの項目では
  // ありません。型の正本はRustで、公開契約は生成したTypeScript宣言です。
  const declarations = readFileSync(new URL("../web-worker/protocol.d.mts", import.meta.url), "utf8");
  const request = declarations.match(/^export type WasmRequest = \{ (.*) \};$/m);
  assert.ok(request, "WasmRequestの宣言がありません");
  assert.match(request[1], /packageVersion: string/);
  assert.doesNotMatch(request[1], /schemaVersion/);
  assert.equal(WORKER_PROTOCOL_SCHEMA_VERSION, PUBLIC_PROTOCOL_SCHEMA_VERSION);
});
