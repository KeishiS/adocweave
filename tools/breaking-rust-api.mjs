import { readFileSync } from "node:fs";

const ROOT = new URL("../", import.meta.url);
/// cargo-semver-checksの出力から機械的に決まる項目。記録には必ず必要です。
const REQUIRED_CHANGE_KEYS = ["crate", "item", "lint", "summary"];
/// 利用者向けの説明と移行手順。0.y系では``release/notes.md``へ書けば足りるため任意とします。
/// 記録した場合はRelease Notesの該当節へそのまま載ります。
const OPTIONAL_CHANGE_KEYS = ["description", "migration"];

const fail = (message) => {
  throw new Error(message);
};

export const breakingFailureKey = ({ crate, lint, item }) => `${crate}\u0000${lint}\u0000${item}`;

export function validateBreakingRustApi(record) {
  if (!record || typeof record !== "object" || Array.isArray(record)) {
    fail("公開Rust APIの破壊的変更記録がobjectではありません");
  }
  const keys = Object.keys(record).sort();
  if (JSON.stringify(keys) !== JSON.stringify(["changes", "releaseVersion", "schemaVersion"])) {
    fail(`公開Rust APIの破壊的変更記録に未知または不足した項目があります：${keys.join("、")}`);
  }
  if (record.schemaVersion !== 1) fail("公開Rust APIの破壊的変更記録のschemaVersionが1ではありません");
  if (typeof record.releaseVersion !== "string" || !/^\d+\.\d+\.\d+$/.test(record.releaseVersion)) {
    fail(`破壊的変更記録のreleaseVersionが X.Y.Z の形式ではありません：${record.releaseVersion}`);
  }
  if (!Array.isArray(record.changes)) fail("公開Rust APIの破壊的変更記録のchangesが配列ではありません");
  const seen = new Set();
  for (const change of record.changes) {
    const changeKeys = change && typeof change === "object" ? Object.keys(change).sort() : [];
    const known = [...REQUIRED_CHANGE_KEYS, ...OPTIONAL_CHANGE_KEYS];
    const unknown = changeKeys.filter((key) => !known.includes(key));
    const missing = REQUIRED_CHANGE_KEYS.filter((key) => !changeKeys.includes(key));
    if (unknown.length > 0 || missing.length > 0) {
      fail(
        `破壊的変更に未知または不足した項目があります：未知=${unknown.join("、") || "なし"}` +
          ` 不足=${missing.join("、") || "なし"}`,
      );
    }
    for (const key of changeKeys) {
      if (typeof change[key] !== "string" || change[key].trim() === "") {
        fail(`破壊的変更の${key}が空です`);
      }
    }
    const key = breakingFailureKey(change);
    if (seen.has(key)) fail(`破壊的変更の記録が重複しています：${key}`);
    seen.add(key);
  }
  return record;
}

export function loadBreakingRustApi() {
  const record = JSON.parse(
    readFileSync(new URL("release/breaking-rust-api.json", ROOT), "utf8"),
  );
  return validateBreakingRustApi(record);
}
