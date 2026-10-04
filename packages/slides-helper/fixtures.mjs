import { readFileSync } from "node:fs";

export function fixture(name) {
  return readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8");
}

export function researchRequest(style = "author-date.csl") {
  const input = JSON.parse(fixture("research.json"));
  input.csl.style = fixture(style);
  return input;
}

export function plainText(inlines) {
  return inlines.map((inline) => inline.kind === "text" ? inline.text : plainText(inline.children)).join("");
}
