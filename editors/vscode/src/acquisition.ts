/**
 * Language Serverの自動取得のうち、実行環境に依存しない判断をまとめます。
 *
 * 版の選択、platformとtargetの対応、asset名の組み立て、およびchecksumの照合は、
 * この場所だけで決めます。
 */
import { createHash } from "node:crypto";

const TAG_PREFIX = "adocweave-lsp/v";
const CHECKSUM_ASSET = "sha256.sum";
const EXECUTABLE = "adocweave-lsp";

/** 配布しているtarget。ここにない環境では自動取得へ進みません。 */
const TARGETS = new Map<string, string>([
  ["linux\0x64", "x86_64-unknown-linux-musl"],
  ["linux\0arm64", "aarch64-unknown-linux-musl"],
  ["darwin\0arm64", "aarch64-apple-darwin"],
  ["win32\0x64", "x86_64-pc-windows-msvc"],
]);

const SUPPORTED_DESCRIPTION =
  "Supported targets are linux x86_64 and aarch64, macOS aarch64, and Windows x86_64";

export interface ReleaseAsset {
  readonly name: string;
  readonly browser_download_url: string;
}

export interface GithubRelease {
  readonly tag_name?: string;
  readonly draft?: boolean;
  readonly prerelease?: boolean;
  readonly assets?: readonly ReleaseAsset[];
}

export interface SelectedRelease {
  readonly version: string;
  readonly assets: readonly ReleaseAsset[];
}

/**
 * 公開済みのLanguage Server releaseから最新版を選びます。
 *
 * 製品ごとにrelease trainを分けているため、tagの接頭辞で絞ってから比較します。
 * draftとprereleaseは選びません。
 */
export function latestServerRelease(body: string): SelectedRelease {
  const releases = JSON.parse(body) as readonly GithubRelease[];
  if (!Array.isArray(releases)) throw new Error("release-list-not-an-array");
  let newest: { order: readonly [number, number, number]; release: SelectedRelease } | undefined;
  for (const release of releases) {
    if (release.draft === true || release.prerelease === true) continue;
    const version = release.tag_name?.startsWith(TAG_PREFIX)
      ? release.tag_name.slice(TAG_PREFIX.length)
      : undefined;
    if (!version) continue;
    const order = stableVersionOrder(version);
    if (!order) continue;
    if (!newest || compareOrder(order, newest.order) > 0) {
      newest = { order, release: { version, assets: release.assets ?? [] } };
    }
  }
  if (!newest) throw new Error("no-published-language-server-release");
  return newest.release;
}

/** 版の比較に使う、stable SemVerの三要素です。prereleaseとbuild metadataは受け付けません。 */
function stableVersionOrder(value: string): readonly [number, number, number] | undefined {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(value);
  if (!match) return undefined;
  return [Number(match[1]), Number(match[2]), Number(match[3])] as const;
}

function compareOrder(
  left: readonly [number, number, number],
  right: readonly [number, number, number],
): number {
  for (let index = 0; index < 3; index += 1) {
    const difference = (left[index] ?? 0) - (right[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

/**
 * Node.jsが報告するplatformとarchitectureを、配布しているtargetへ対応させます。
 *
 * Intel macOSとWindows ARM64向けのarchiveは配布していません。
 */
export function targetTriple(platform: NodeJS.Platform, architecture: string): string {
  const target = TARGETS.get(`${platform}\0${architecture}`);
  if (!target) {
    throw new Error(`unsupported-platform: ${platform} ${architecture}. ${SUPPORTED_DESCRIPTION}.`);
  }
  return target;
}

export function assetName(target: string): string {
  return `${EXECUTABLE}-${target}.zip`;
}

export function checksumAssetName(): string {
  return CHECKSUM_ASSET;
}

/** 取得した版を置くディレクトリ名です。版とtargetを含め、混在しても取り違えません。 */
export function versionDirectory(version: string, target: string): string {
  return `${EXECUTABLE}-${version}-${target}`;
}

export function executableName(platform: NodeJS.Platform): string {
  return platform === "win32" ? `${EXECUTABLE}.exe` : EXECUTABLE;
}

/**
 * `sha256.sum`から対象fileの期待値を取り出します。
 *
 * 形式は`<hex>  <name>`です。対象の行がない場合は失敗させます。照合できない
 * archiveを、検証したものとして扱わないためです。
 */
export function expectedChecksum(sums: string, fileName: string): string {
  for (const line of sums.split("\n")) {
    const match = /^([0-9a-f]{64}) [ *](.+)$/.exec(line.trimEnd());
    if (match && match[2] === fileName) return match[1] as string;
  }
  throw new Error(`checksum-entry-missing: ${fileName}`);
}

/**
 * 取得したarchiveのchecksumを照合します。
 *
 * 一致しない場合は展開せずに失敗させます。改変された、または壊れたarchiveを
 * 展開しないためです。
 */
export function verifyChecksum(archive: Uint8Array, expected: string, fileName: string): void {
  const actual = createHash("sha256").update(archive).digest("hex");
  if (actual !== expected) {
    throw new Error(`checksum-mismatch: ${fileName} expected ${expected} actual ${actual}`);
  }
}
