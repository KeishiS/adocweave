import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("../", import.meta.url));

function escapeForPattern(value) {
  return value.replaceAll(/[.*+?^${}()|[\]\\]/gu, String.raw`\$&`);
}

// npmへ公開する製品は、配布計画のversionSourceがpackage.jsonを指し、
// そのmanifestがpublic公開を明示し、成果物がnpmのtarballである製品に限る。
export function npmPackageProduct(productId, root = ROOT) {
  const plan = JSON.parse(readFileSync(resolve(root, "release/distribution-plan.json"), "utf8"));
  const product = plan.products.find((entry) => entry.product === productId);
  if (product === undefined) {
    throw new Error(`配布計画に製品がありません: ${productId}`);
  }
  const [manifestPath, field] = product.versionSource.split("#");
  if (!manifestPath.endsWith("package.json") || field !== "version" ||
      !product.assetName.endsWith(".tgz")) {
    throw new Error(`npmへ公開する成果物を持つ製品ではありません: ${productId}`);
  }
  const manifest = JSON.parse(readFileSync(resolve(root, manifestPath), "utf8"));
  if (manifest.private === true || manifest.publishConfig?.access !== "public") {
    throw new Error(`npmへ公開する設定になっていません: ${productId}`);
  }
  return Object.freeze({
    product: productId,
    assetName: product.assetName,
    packageName: manifest.name
  });
}

export function resolvePackageAsset(productId, directory, root = ROOT) {
  const product = npmPackageProduct(productId, root);
  const pattern = new RegExp(
    `^${escapeForPattern(product.assetName).replace(String.raw`\{version\}`, "(.+)")}$`,
    "u"
  );
  const matches = readdirSync(directory).filter((name) => pattern.test(name));
  if (matches.length !== 1) {
    throw new Error(
      `${productId}のnpm成果物を一つに定められません: ${matches.length}件 (${directory})`
    );
  }
  return Object.freeze({
    ...product,
    path: join(directory, matches[0]),
    version: pattern.exec(matches[0])[1]
  });
}

export function tarballIntegrity(path) {
  return `sha512-${createHash("sha512").update(readFileSync(path)).digest("base64")}`;
}

// 公開したbyte列がGitHub Releaseの成果物と同じであることを、registryが記録した
// integrityと突き合わせて確かめる。異なる場合は公開経路のどこかで作り直している。
export async function verifyPublishedPackage(asset, fetchJson = defaultFetchJson) {
  const metadata = await fetchJson(
    `https://registry.npmjs.org/${asset.packageName}/${asset.version}`
  );
  if (metadata?.version !== asset.version) {
    throw new Error(`npmに${asset.packageName}@${asset.version}が見つかりません`);
  }
  const expected = tarballIntegrity(asset.path);
  if (metadata.dist?.integrity !== expected) {
    throw new Error(
      `npmの${asset.packageName}@${asset.version}がReleaseの成果物と一致しません`
    );
  }
  return Object.freeze({ packageName: asset.packageName, version: asset.version });
}

async function defaultFetchJson(url) {
  const response = await fetch(url, { headers: { accept: "application/json" } });
  if (!response.ok) throw new Error(`npm registryへの問い合わせに失敗しました: ${response.status}`);
  return response.json();
}

async function main() {
  const [command, productId, target] = process.argv.slice(2);
  if (command === "resolve" && productId && target) {
    process.stdout.write(`${resolvePackageAsset(productId, target).path}\n`);
    return;
  }
  if (command === "verify-published" && productId && target) {
    const asset = resolvePackageAsset(productId, target);
    const published = await verifyPublishedPackage(asset);
    process.stdout.write(`Published ${published.packageName} ${published.version} to npm\n`);
    return;
  }
  throw new Error("使用方法：node tools/npm-package-asset.mjs resolve|verify-published PRODUCT DIRECTORY");
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
