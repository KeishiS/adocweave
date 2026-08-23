import { readFileSync } from "node:fs";

const ID = /^RUSTSEC-\d{4}-\d{4}$/;
const REASON = /^理由: (.+); 期限: (\d{4}-\d{2}-\d{2}); Issue: (https:\/\/github\.com\/KeishiS\/adocweave\/issues\/\d+)$/;

export function validateAdvisoryExceptions(config, today) {
  const entries = config.advisories?.ignore ?? [];
  if (!Array.isArray(entries)) throw new Error("advisories.ignoreは配列で指定してください");
  for (const entry of entries) {
    if (!entry || typeof entry !== "object" || !ID.test(entry.id ?? "")) {
      throw new Error("advisory例外にはRustSec IDが必要です");
    }
    const match = REASON.exec(entry.reason ?? "");
    if (!match || match[1].trim().length === 0) {
      throw new Error(`${entry.id}の例外には理由、期限および追跡Issueが必要です`);
    }
    if (match[2] <= today) throw new Error(`${entry.id}の例外期限が切れています`);
  }
}

export function main() {
  const config = JSON.parse(readFileSync(0, "utf8"));
  validateAdvisoryExceptions(config, new Date().toISOString().slice(0, 10));
  process.stdout.write("advisory例外を検証しました。\n");
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
