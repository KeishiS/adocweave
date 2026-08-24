import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const ROOT = new URL("../", import.meta.url);
const STABLE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
export const PRODUCT_IDS = ["cli", "lsp", "wasm", "textlint", "vscode", "zed"];
const IGNORED_DIRECTORIES = new Set([
  ".agents",
  ".git",
  ".vscode-test",
  "node_modules",
  "target",
]);

function fail(message) {
  throw new Error(message);
}

function exactKeys(value, keys, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label}はobjectである必要があります`);
  }
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    fail(`${label}に不足、余分または未知のfieldがあります`);
  }
}

function safePath(path, label) {
  if (
    typeof path !== "string" ||
    path.length === 0 ||
    path.startsWith("/") ||
    path.includes("\\") ||
    path.split("/").some((part) => part === "" || part === "." || part === "..")
  ) {
    fail(`${label}のpathが安全なrepository相対pathではありません`);
  }
}

function occurrences(source, literal) {
  if (literal.length === 0) fail("空文字列は検索できません");
  let count = 0;
  let offset = 0;
  while ((offset = source.indexOf(literal, offset)) !== -1) {
    count += 1;
    offset += literal.length;
  }
  return count;
}

function versionTemplate(template, label) {
  if (
    typeof template !== "string" ||
    occurrences(template, "{version}") !== 1
  ) {
    fail(`${label}のtemplateは{version}を1回だけ含める必要があります`);
  }
}

/// `count`の代わりに書ける宣言。そのfileに現れるversion記録をすべて置き換えます。
///
/// 件数を数で宣言すると、記録を1つ足すたびにこのregistryの数値を手で直す必要があります。
/// Release Notesのような散文では文面を変えるたびに件数が動き、置換漏れではなく数え直しの
/// 手間だけが増えていました。fileの中のversion記録をすべて管理する対象では、件数そのものに
/// 意味がないため、1件以上あることだけを求めます。
export const ALL_OCCURRENCES = "all";

function positiveCount(count, label, allowZero = false) {
  if (
    !Number.isInteger(count) ||
    count < (allowZero ? 0 : 1)
  ) {
    fail(`${label}のcountが不正です`);
  }
}

function validateProduct(product, label) {
  exactKeys(product, ["id", "authority", "targets", "generators"], label);
  if (!PRODUCT_IDS.includes(product.id)) fail(`${label}.idが未対応です：${product.id}`);
  exactKeys(product.authority, ["type", "path", "template", "count"], `${label}.authority`);
  if (product.authority.type !== "literal") {
    fail(`${label}.authorityのtypeはliteralである必要があります`);
  }
  safePath(product.authority.path, `${label}.authority`);
  versionTemplate(product.authority.template, `${label}.authority`);
  positiveCount(product.authority.count, `${label}.authority`);

  if (!Array.isArray(product.targets)) fail(`${label}.targetsはarrayである必要があります`);
  const locators = new Set();
  for (const [index, target] of product.targets.entries()) {
    const targetLabel = `${label}.targets[${index}]`;
    if (target?.type === "literal") {
      exactKeys(target, ["type", "path", "template", "count"], targetLabel);
      versionTemplate(target.template, targetLabel);
      if (target.count !== ALL_OCCURRENCES) positiveCount(target.count, targetLabel);
    } else if (target?.type === "cargo-lock") {
      exactKeys(target, ["type", "path", "packages"], targetLabel);
      if (
        !Array.isArray(target.packages) ||
        target.packages.length === 0 ||
        target.packages.some(
          (name) => typeof name !== "string" || !/^[a-z0-9-]+$/.test(name),
        ) ||
        new Set(target.packages).size !== target.packages.length
      ) {
        fail(`${targetLabel}.packagesは重複のないpackage名である必要があります`);
      }
    } else {
      fail(`${targetLabel}.typeはliteralまたはcargo-lockである必要があります`);
    }
    safePath(target.path, targetLabel);
    const identity = target.type === "literal"
      ? `${target.path}\0literal\0${target.template}`
      : `${target.path}\0cargo-lock\0${target.packages.join(",")}`;
    if (locators.has(identity)) fail(`${targetLabel}は重複しています`);
    locators.add(identity);
  }
  const authorityIdentity =
    `${product.authority.path}\0literal\0${product.authority.template}`;
  if (locators.has(authorityIdentity)) {
    fail(`${label}.authorityをtargetsへ重複登録できません`);
  }

  if (!Array.isArray(product.generators)) fail(`${label}.generatorsはarrayである必要があります`);
  const generatorIds = new Set();
  const outputPaths = new Set();
  for (const [index, generator] of product.generators.entries()) {
    const generatorLabel = `${label}.generators[${index}]`;
    exactKeys(generator, ["id", "outputs"], generatorLabel);
    if (generator.id !== "protocol") {
      fail(`${generatorLabel}のgenerator IDは許可されていません`);
    }
    if (generatorIds.has(generator.id)) fail(`${generatorLabel}のIDは重複しています`);
    generatorIds.add(generator.id);
    if (!Array.isArray(generator.outputs) || generator.outputs.length === 0) {
      fail(`${generatorLabel}.outputsは空でないarrayである必要があります`);
    }
    for (const [outputIndex, output] of generator.outputs.entries()) {
      const outputLabel = `${generatorLabel}.outputs[${outputIndex}]`;
      exactKeys(output, ["path"], outputLabel);
      safePath(output.path, outputLabel);
      if (outputPaths.has(output.path)) {
        fail(`${outputLabel}のpathは重複しています`);
      }
      outputPaths.add(output.path);
    }
  }
}

export function validateRegistry(registry) {
  exactKeys(registry, ["schemaVersion", "products"], "version同期registry");
  if (registry.schemaVersion !== 2) {
    fail("version同期registryのschemaVersionは2である必要があります");
  }
  if (!Array.isArray(registry.products)) fail("productsはarrayである必要があります");
  registry.products.forEach((product, index) => validateProduct(product, `products[${index}]`));
  const ids = registry.products.map(({ id }) => id);
  if (new Set(ids).size !== ids.length) fail("product IDが重複しています");
  if (JSON.stringify([...ids].sort()) !== JSON.stringify([...PRODUCT_IDS].sort())) {
    fail(`productsは${PRODUCT_IDS.join("、")}を1件ずつ含める必要があります`);
  }
  return registry;
}

function absolute(root, path) {
  return resolve(fileURLToPath(root), path);
}

function read(root, path) {
  const url = absolute(root, path);
  if (!existsSync(url) || !statSync(url).isFile()) {
    fail(`管理対象fileがありません：${path}`);
  }
  return readFileSync(url, "utf8");
}

function render(template, version) {
  return template.replace("{version}", version);
}

function compareVersions(left, right) {
  const leftParts = left.split(".").map(BigInt);
  const rightParts = right.split(".").map(BigInt);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] < rightParts[index]) return -1;
    if (leftParts[index] > rightParts[index]) return 1;
  }
  return 0;
}

function validateLocator(root, locator, version, label) {
  if (locator.type === "cargo-lock") {
    cargoLockBlocks(read(root, locator.path), locator, version, label);
    return;
  }
  const literal = render(locator.template, version);
  const actual = occurrences(read(root, locator.path), literal);
  if (locator.count === ALL_OCCURRENCES) {
    if (actual === 0) {
      fail(`${label} ${locator.path} にversion記録がありません`);
    }
    return;
  }
  if (actual !== locator.count) {
    fail(
      `${label} ${locator.path} のversion記録数が不正です：期待${locator.count}件、実際${actual}件`,
    );
  }
}

function cargoLockBlocks(source, locator, version, label) {
  const blocks = source.split(/\n\n/);
  const selected = [];
  for (const packageName of locator.packages) {
    const matches = blocks.filter(
      (block) =>
        new RegExp(`^name = "${packageName}"$`, "m").test(block),
    );
    if (matches.length !== 1) {
      fail(
        `${label} ${locator.path} のpackage ${packageName}が一意ではありません`,
      );
    }
    const block = matches[0];
    if (/^source = /m.test(block)) {
      fail(`${label} ${locator.path} のpackage ${packageName}はlocal packageではありません`);
    }
    const expected = `version = "${version}"`;
    if (occurrences(block, expected) !== 1) {
      fail(
        `${label} ${locator.path} のpackage ${packageName}がversion ${version}ではありません`,
      );
    }
    selected.push(block);
  }
  return { blocks, selected };
}

function walkedSourceFiles(root, directory = "", result = new Map()) {
  const url = absolute(root, directory || "./");
  for (const entry of readdirSync(url, { withFileTypes: true })) {
    if (IGNORED_DIRECTORIES.has(entry.name)) continue;
    const path = directory ? `${directory}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      walkedSourceFiles(root, path, result);
    } else if (entry.isFile()) {
      const buffer = readFileSync(absolute(root, path));
      if (!buffer.includes(0)) result.set(path, buffer.toString("utf8"));
    }
  }
  return result;
}

function sourceFiles(root) {
  const tracked = spawnSync(
    "git",
    [
      "-C",
      fileURLToPath(root),
      "ls-files",
      "--cached",
      "--others",
      "--exclude-standard",
      "-z",
    ],
    { encoding: "utf8" },
  );
  if (tracked.status !== 0) return walkedSourceFiles(root);
  const result = new Map();
  for (const path of tracked.stdout.split("\0").filter(Boolean)) {
    if (IGNORED_DIRECTORIES.has(path.split("/", 1)[0])) continue;
    const file = absolute(root, path);
    if (!existsSync(file) || !statSync(file).isFile()) continue;
    const buffer = readFileSync(file);
    if (!buffer.includes(0)) result.set(path, buffer.toString("utf8"));
  }
  return result;
}

// 版表記の管理は、registryへ列挙したfileとパターンだけを対象とします。以前は
// リポジトリ全体を走査し、登録外のfileに版文字列があれば失敗させていました。登録漏れは
// 見つかる一方で、cargo-distの版やlockfileの第三者package、旧版を含むtest fixtureまで
// 「保持対象」として登録し続ける必要があり、上流の値が動くたびに人が追従していました。
// 主要なmanifestの版はrelease-contractがrelease manifestと突き合わせるため、
// 二重の検査を維持する価値より保守の負担が上回ると判断しています。

function validateState(root, product, version) {
  validateLocator(root, product.authority, version, `${product.id}.authority`);
  for (const [index, target] of product.targets.entries()) {
    validateLocator(root, target, version, `${product.id}.targets[${index}]`);
  }
}

function run(command, args, root) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    fail(`${command} ${args.join(" ")} が終了code ${result.status}で失敗しました`);
  }
}

export function runRepositoryGenerator({ id, mode, root }) {
  if (id === "protocol") {
    // TypeScript宣言はRustのwire型から書き出します。checkでも同じtestを実行し、
    // 差分が出た場合は呼出側が管理対象外の変更として検出します。
    run(
      "cargo",
      [
        "test",
        "--locked",
        "-p",
        "adocweave-wasm",
        "--features",
        "ts-rs",
        "export_bindings",
      ],
      root,
    );
    return;
  }
  fail(`未知のgeneratorです：${id}`);
}

function changedPaths(before, after) {
  const paths = new Set([...before.keys(), ...after.keys()]);
  return [...paths].filter((path) => before.get(path) !== after.get(path));
}

function restore(root, before, after) {
  for (const path of changedPaths(before, after)) {
    if (before.has(path)) {
      writeFileSync(absolute(root, path), before.get(path));
    } else if (existsSync(absolute(root, path))) {
      unlinkSync(absolute(root, path));
    }
  }
}

function updateLocators(root, locators, current, next) {
  const updated = new Map();
  for (const locator of locators) {
    const source = updated.get(locator.path) ?? read(root, locator.path);
    if (locator.type === "cargo-lock") {
      const { blocks, selected } = cargoLockBlocks(
        source,
        locator,
        current,
        locator.path,
      );
      const selectedSet = new Set(selected);
      updated.set(
        locator.path,
        blocks
          .map((block) =>
            selectedSet.has(block)
              ? block.replace(
                  `version = "${current}"`,
                  `version = "${next}"`,
                )
              : block,
          )
          .join("\n\n"),
      );
      continue;
    }
    const from = render(locator.template, current);
    const to = render(locator.template, next);
    const actual = occurrences(source, from);
    if (locator.count === ALL_OCCURRENCES ? actual === 0 : actual !== locator.count) {
      fail(
        `${locator.path}を更新できません：期待${locator.count}件、実際${actual}件`,
      );
    }
    updated.set(locator.path, source.split(from).join(to));
  }
  for (const [path, source] of updated) {
    writeFileSync(absolute(root, path), source);
  }
}

export function syncReleaseVersion({
  root = ROOT,
  mode,
  product: productId,
  version,
  registry,
  runGenerator = runRepositoryGenerator,
}) {
  if (!(root instanceof URL) || root.protocol !== "file:") {
    fail("rootはfile URLである必要があります");
  }
  validateRegistry(registry);
  const product = registry.products.find(({ id }) => id === productId);
  if (!product) fail(`未対応のproductです：${productId ?? "<missing>"}`);
  const authoritySource = read(root, product.authority.path);
  const authorityPattern = product.authority.template
    .replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
    .replace("\\{version\\}", "([0-9]+\\.[0-9]+\\.[0-9]+)");
  const matches = [...authoritySource.matchAll(new RegExp(authorityPattern, "g"))];
  if (matches.length !== product.authority.count) {
    fail(`${product.id}のauthorityから現在のversionを一意に取得できません`);
  }
  const current = matches[0][1];
  if (!STABLE_VERSION.test(current)) fail(`authorityのversionが不正です：${current}`);
  if (mode !== "check" && mode !== "update") fail(`未知のmodeです：${mode}`);
  if (mode === "update" && !STABLE_VERSION.test(version ?? "")) {
    fail(`更新先versionがstable SemVerではありません：${version ?? "<missing>"}`);
  }
  if (
    mode === "update" &&
    version !== current &&
    compareVersions(version, current) <= 0
  ) {
    fail(`更新先versionは現在のversionより大きい必要があります：${version}`);
  }

  validateState(root, product, current);
  const before = sourceFiles(root);
  if (mode === "check") {
    try {
      for (const generator of product.generators) {
        runGenerator({ id: generator.id, mode, root, generator });
      }
      const after = sourceFiles(root);
      const changed = changedPaths(before, after);
      if (changed.length > 0) {
        fail(`--checkがfileを変更しました：${changed.join(", ")}`);
      }
    } catch (error) {
      restore(root, before, sourceFiles(root));
      throw error;
    }
    process.stdout.write(`${product.id}のrelease version同期を検査しました：${current}\n`);
    return { current, version: current, changed: [] };
  }

  if (version === current) {
    process.stdout.write(`${product.id}のrelease versionはすでに${current}です\n`);
    return { current, version, changed: [] };
  }

  try {
    updateLocators(
      root,
      [...product.targets, product.authority],
      current,
      version,
    );
    for (const generator of product.generators) {
      runGenerator({
        id: generator.id,
        mode,
        root,
        generator,
        current,
        version,
      });
    }
    validateState(root, product, version);
    const after = sourceFiles(root);
    const allowed = new Set([
      product.authority.path,
      ...product.targets.map(({ path }) => path),
      ...product.generators.flatMap(({ outputs }) =>
        outputs.map(({ path }) => path)),
    ]);
    const unexpected = changedPaths(before, after).filter(
      (path) => !allowed.has(path),
    );
    if (unexpected.length > 0) {
      fail(`同期処理が管理対象外を変更しました：${unexpected.join(", ")}`);
    }
    process.stdout.write(`${product.id}のrelease versionを${current}から${version}へ同期しました\n`);
    return { current, version, changed: changedPaths(before, after) };
  } catch (error) {
    restore(root, before, sourceFiles(root));
    throw error;
  }
}

export function parseReleaseVersionArguments(args) {
  if (args.length === 1 && args[0] === "--check") {
    return { mode: "check", product: undefined, version: undefined };
  }
  if (args.length === 3 && args[0] === "--product" && args[2] === "--check") {
    return { mode: "check", product: args[1], version: undefined };
  }
  if (
    args.length === 4 &&
    args[0] === "--product" &&
    args[2] === "--version"
  ) {
    return { mode: "update", product: args[1], version: args[3] };
  }
  fail(
    "使用方法：node tools/sync-release-version.mjs --check | --product PRODUCT --check | --product PRODUCT --version X.Y.Z",
  );
}

export function main(args) {
  const options = parseReleaseVersionArguments(args);
  const registry = JSON.parse(
    readFileSync(new URL("release/version-sync.json", ROOT), "utf8"),
  );
  validateRegistry(registry);
  if (options.mode === "check" && options.product === undefined) {
    for (const { id: product } of registry.products) {
      syncReleaseVersion({ ...options, product, registry });
    }
    return;
  }
  syncReleaseVersion({ ...options, registry });
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
