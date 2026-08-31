const catalogKeys = ["forbiddenTerms", "schemaVersion", "warningTerms"];
const entryKeys = ["id", "message", "term"];

function assertRecord(value, name) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${name}はオブジェクトで指定してください。`);
  }
}

function assertExactKeys(value, expected, name) {
  const actual = Object.keys(value).sort();
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new Error(`${name}の項目がschemaと一致しません: ${actual.join(", ")}`);
  }
}

function assertNonEmptyString(value, name) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${name}は空でない文字列で指定してください。`);
  }
}

function validateCatalog(catalog) {
  assertRecord(catalog, "日本語用語集");
  assertExactKeys(catalog, catalogKeys, "日本語用語集");
  if (
    catalog.schemaVersion !== 3 ||
    !Array.isArray(catalog.forbiddenTerms) ||
    !Array.isArray(catalog.warningTerms)
  ) {
    throw new Error("日本語用語集のschemaVersionを解釈できません。");
  }

  const ids = new Set();
  const validateEntries = (entries, key) => Object.freeze(
    entries.map((entry, index) => {
      const name = `日本語用語集の${key}[${index}]`;
      assertRecord(entry, name);
      assertExactKeys(entry, entryKeys, name);
      assertNonEmptyString(entry.id, `${name}.id`);
      assertNonEmptyString(entry.term, `${name}.term`);
      assertNonEmptyString(entry.message, `${name}.message`);
      if (ids.has(entry.id)) {
        throw new Error(`日本語用語集のidが重複しています: ${entry.id}`);
      }
      ids.add(entry.id);
      return Object.freeze({
        id: entry.id,
        term: entry.term,
        message: entry.message
      });
    })
  );
  return Object.freeze({
    forbiddenTerms: validateEntries(catalog.forbiddenTerms, "forbiddenTerms"),
    warningTerms: validateEntries(catalog.warningTerms, "warningTerms")
  });
}

function createRule(entries, options) {
  const { ignoreStandaloneUrls = false } = options;
  return (context) => {
    const { Syntax, RuleError, locator, report } = context;
    return {
      [Syntax.Str](node) {
        if (ignoreStandaloneUrls && /^(?:https?:\/\/|mailto:|www\.)\S+$/iu.test(node.value)) {
          return;
        }
        for (const entry of entries) {
          let start = 0;
          while (start <= node.value.length) {
            const index = node.value.indexOf(entry.term, start);
            if (index === -1) {
              break;
            }
            report(
              node,
              new RuleError(`${entry.message} [${entry.id}]`, {
                padding: locator.range([index, index + entry.term.length])
              })
            );
            start = index + entry.term.length;
          }
        }
      }
    };
  };
}

export function createTerminologyRules(catalog, options = {}) {
  const entries = validateCatalog(catalog);
  return Object.freeze([
    Object.freeze({
      ruleId: "adocweave-terminology",
      rule: createRule(entries.forbiddenTerms, options)
    }),
    Object.freeze({
      ruleId: "adocweave-terminology-warning",
      rule: createRule(entries.warningTerms, options),
      options: Object.freeze({ severity: "warning" })
    })
  ]);
}
