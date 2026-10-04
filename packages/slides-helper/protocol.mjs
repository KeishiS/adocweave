export const LIMITS = Object.freeze({
  inputBytes: 4 * 1024 * 1024,
  outputBytes: 32 * 1024 * 1024,
  equations: 1024,
  texBytes: 16 * 1024,
  citations: 1024,
  citationItems: 64,
  bibliographyItems: 4096,
  macros: 64,
  macroBytes: 4096,
  xmlBytes: 512 * 1024,
  noticeTextBytes: 64 * 1024,
  noticeBytes: 256 * 1024,
});

export const SCOPES = Object.freeze(["body", "notes"]);

export class RequestError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

function requireCondition(condition, message, code = "invalid-request") {
  if (!condition) throw new RequestError(code, message);
}

function object(value, path, allowed, required = allowed) {
  requireCondition(value !== null && typeof value === "object" && !Array.isArray(value), `${path} must be an object.`);
  for (const key of Object.keys(value)) {
    requireCondition(allowed.includes(key), `${path} has an unsupported field: ${key}.`);
  }
  for (const key of required) {
    requireCondition(Object.hasOwn(value, key), `${path}.${key} is required.`);
  }
}

function text(value, path, bytes, nonempty = false) {
  requireCondition(typeof value === "string" && (!nonempty || value.length > 0), `${path} must be ${nonempty ? "a nonempty" : "a"} string.`);
  requireCondition(Buffer.byteLength(value) <= bytes, `${path} exceeds its byte limit.`, "input-limit");
}

function list(value, path, maximum) {
  requireCondition(Array.isArray(value), `${path} must be an array.`);
  requireCondition(value.length <= maximum, `${path} exceeds its item limit.`, "input-limit");
}

function boundedJson(value, depth = 0) {
  requireCondition(depth <= 32, "CSL items exceed the nesting limit.", "input-limit");
  if (value === null || typeof value !== "object") {
    requireCondition(["string", "number", "boolean"].includes(typeof value) || value === null, "CSL items must contain JSON values.");
    if (typeof value === "number") requireCondition(Number.isFinite(value), "CSL numeric values must be finite.");
    return;
  }
  for (const [key, child] of Object.entries(value)) {
    requireCondition(!["__proto__", "prototype", "constructor"].includes(key), `CSL items contain a reserved field: ${key}.`);
    boundedJson(child, depth + 1);
  }
}

export function validateRequest(request) {
  object(request, "request", ["schemaVersion", "eqnums", "scopes", "macros", "csl"], ["schemaVersion", "eqnums", "scopes"]);
  requireCondition(request.schemaVersion === 1, "Unsupported schemaVersion; expected 1.");
  requireCondition(["none", "ams", "all"].includes(request.eqnums), "eqnums must be none, ams, or all.");
  object(request.scopes, "scopes", SCOPES);
  let equationCount = 0;
  let citationCount = 0;
  for (const scope of SCOPES) {
    const input = request.scopes[scope];
    object(input, `scopes.${scope}`, ["equations", "citations"]);
    list(input.equations, `${scope}.equations`, LIMITS.equations);
    list(input.citations, `${scope}.citations`, LIMITS.citations);
    equationCount += input.equations.length;
    citationCount += input.citations.length;
    const keys = new Set();
    const key = (value) => {
      requireCondition(typeof value === "string" && /^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(value), "A result key must be an ASCII identifier of at most 64 characters.");
      requireCondition(!keys.has(value), `Duplicate key in ${scope}: ${value}.`);
      keys.add(value);
    };
    for (const equation of input.equations) {
      object(equation, "equation", ["key", "tex", "display"]);
      key(equation.key);
      text(equation.tex, "equation.tex", LIMITS.texBytes);
      requireCondition(typeof equation.display === "boolean", "equation.display must be a boolean.");
    }
    for (const citation of input.citations) {
      object(citation, "citation", ["key", "items"]);
      key(citation.key);
      list(citation.items, "citation.items", LIMITS.citationItems);
      requireCondition(citation.items.length > 0, "citation.items must not be empty.");
      for (const item of citation.items) {
        object(item, "citation item", ["id", "locator", "label", "prefix", "suffix", "suppressAuthor", "authorOnly"], ["id"]);
        for (const field of ["id", "locator", "label", "prefix", "suffix"]) {
          if (Object.hasOwn(item, field)) text(item[field], `citation item.${field}`, LIMITS.macroBytes, field === "id");
        }
        for (const field of ["suppressAuthor", "authorOnly"]) {
          if (Object.hasOwn(item, field)) requireCondition(typeof item[field] === "boolean", `citation item.${field} must be a boolean.`);
        }
        requireCondition(!(item.authorOnly && item.suppressAuthor), "A citation item cannot both suppress and select its author.");
      }
    }
  }
  requireCondition(equationCount <= LIMITS.equations && citationCount <= LIMITS.citations, "The request exceeds the equation or citation count limit.", "input-limit");
  if (Object.hasOwn(request, "macros")) {
    list(request.macros, "macros", LIMITS.macros);
    const names = new Set();
    for (const macro of request.macros) {
      object(macro, "macro", ["name", "definition", "arguments", "default"], ["name", "definition"]);
      requireCondition(typeof macro.name === "string" && /^[A-Za-z]{1,64}$/.test(macro.name) && !["constructor", "prototype"].includes(macro.name), "A macro name must contain 1 to 64 ASCII letters.");
      requireCondition(!names.has(macro.name), `Duplicate macro: ${macro.name}.`);
      names.add(macro.name);
      text(macro.definition, "macro.definition", LIMITS.macroBytes);
      requireCondition(macro.arguments === undefined || (Number.isInteger(macro.arguments) && macro.arguments >= 0 && macro.arguments <= 9), "macro.arguments must be an integer from 0 to 9.");
      if (Object.hasOwn(macro, "default")) {
        text(macro.default, "macro.default", LIMITS.macroBytes);
        requireCondition(macro.arguments > 0, "An optional macro argument requires at least one argument.");
      }
    }
  }
  if (Object.hasOwn(request, "csl")) {
    object(request.csl, "csl", ["items", "style", "locale"]);
    text(request.csl.style, "csl.style", LIMITS.xmlBytes, true);
    text(request.csl.locale, "csl.locale", LIMITS.xmlBytes, true);
    list(request.csl.items, "csl.items", LIMITS.bibliographyItems);
    const ids = new Set();
    for (const item of request.csl.items) {
      requireCondition(item !== null && typeof item === "object" && !Array.isArray(item), "A CSL item must be an object.");
      text(item.id, "CSL item.id", LIMITS.macroBytes, true);
      requireCondition(!ids.has(item.id), `Duplicate CSL item id: ${item.id}.`);
      ids.add(item.id);
      boundedJson(item);
    }
  }
  requireCondition(citationCount === 0 || request.csl !== undefined, "Citations require csl items, style, and locale.");
  return request;
}

export function emptyResponse() {
  return {
    schemaVersion: 1,
    notices: { math: null, citations: null },
    scopes: Object.fromEntries(SCOPES.map((scope) => [scope, { equations: [], citations: [], bibliography: [] }])),
    diagnostics: [],
  };
}

export function diagnostic(scope, key, code, message, severity = "error") {
  return { scope, key, severity, code, message };
}

export function encodeResponse(response) {
  let json = JSON.stringify(response);
  if (Buffer.byteLength(json) > LIMITS.outputBytes) {
    response = emptyResponse();
    response.diagnostics.push(diagnostic(null, null, "output-limit", "The response exceeds the output byte limit."));
    json = JSON.stringify(response);
  }
  return { json, failed: response.diagnostics.some(({ severity }) => severity === "error") };
}
