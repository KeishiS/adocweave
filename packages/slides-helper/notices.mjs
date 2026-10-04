import { readFileSync } from "node:fs";

import { LIMITS, RequestError } from "./protocol.mjs";

const files = {
  math: {
    fontAttribution: "mathjax-newcm-font-notices.txt",
    fontLicense: "GUST-FONT-LICENSE.txt",
    lpplLicense: "LPPL-1.3c.txt",
    mathjaxLicense: "@mathjax-src.txt",
  },
  citations: { attribution: "citeproc-attribution.txt", license: "citeproc-CPAL.txt" },
};

export function loadNotices(usesMath, usesCitations) {
  const result = { math: null, citations: null };
  let bytes = 0;
  for (const [kind, used] of [["math", usesMath], ["citations", usesCitations]]) {
    if (!used) continue;
    result[kind] = {};
    for (const [field, filename] of Object.entries(files[kind])) {
      const data = readFileSync(new URL(`./licenses/${filename}`, import.meta.url));
      bytes += data.length;
      if (data.length > LIMITS.noticeTextBytes || bytes > LIMITS.noticeBytes) {
        throw new RequestError("notice-limit", "The bundled license notices exceed their byte limit.");
      }
      result[kind][field] = new TextDecoder("utf-8", { fatal: true }).decode(data);
    }
  }
  return result;
}
