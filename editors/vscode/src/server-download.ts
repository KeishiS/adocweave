/**
 * GitHub ReleaseからLanguage Serverを取得します。
 *
 * 判断は`acquisition.ts`が持ち、この場所はnetworkとfilesystemの操作だけを行います。
 * 展開はchecksumの照合に成功したあとにだけ実行します。
 */
import { chmod, mkdir, mkdtemp, readdir, rename, rm, stat, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";

import { unzipSync } from "fflate";

import {
  assetName,
  checksumAssetName,
  executableName,
  latestServerRelease,
  type SelectedRelease,
  targetTriple,
  expectedChecksum,
  verifyChecksum,
  versionDirectory,
} from "./acquisition.js";

const RELEASES_URL = "https://api.github.com/repos/KeishiS/adocweave/releases?per_page=100";
const REQUEST_HEADERS = {
  Accept: "application/vnd.github+json",
  "User-Agent": "adocweave-vscode-client",
};

export interface DownloadDependencies {
  readonly fetchText: (url: string) => Promise<string>;
  readonly fetchBytes: (url: string) => Promise<Uint8Array>;
  readonly platform: NodeJS.Platform;
  readonly architecture: string;
  readonly onProgress?: (message: string) => void;
}

const defaultDependencies: Omit<DownloadDependencies, "onProgress"> = {
  fetchText: async (url) => {
    const response = await fetch(url, { headers: REQUEST_HEADERS });
    if (!response.ok) throw new Error(`download-failed: ${url} ${response.status}`);
    return response.text();
  },
  fetchBytes: async (url) => {
    const response = await fetch(url, { headers: REQUEST_HEADERS });
    if (!response.ok) throw new Error(`download-failed: ${url} ${response.status}`);
    return new Uint8Array(await response.arrayBuffer());
  },
  platform: process.platform,
  architecture: process.arch,
};

async function isFile(path: string): Promise<boolean> {
  try {
    return (await stat(path)).isFile();
  } catch {
    return false;
  }
}

function assetUrl(release: SelectedRelease, name: string): string {
  const asset = release.assets.find((candidate) => candidate.name === name);
  if (!asset) throw new Error(`release-asset-missing: ${name} in ${release.version}`);
  return asset.browser_download_url;
}

/**
 * Language Serverを取得し、実行ファイルの絶対pathを返します。
 *
 * 同じ版を取得済みであればdownloadしません。取得はまず一時ディレクトリへ展開し、
 * 成功したときだけ目的の場所へ移します。途中で失敗した状態を、取得済みとして
 * 扱わないためです。
 */
export async function downloadServer(
  storageDirectory: string,
  overrides: Partial<DownloadDependencies> = {},
): Promise<string> {
  const dependencies = { ...defaultDependencies, ...overrides };
  const target = targetTriple(dependencies.platform, dependencies.architecture);
  const release = latestServerRelease(await dependencies.fetchText(RELEASES_URL));
  const directory = join(storageDirectory, versionDirectory(release.version, target));
  const executable = join(directory, executableName(dependencies.platform));
  if (await isFile(executable)) return executable;

  const archiveName = assetName(target);
  dependencies.onProgress?.(`Downloading the Language Server ${release.version} (${target}).`);
  const sums = await dependencies.fetchText(assetUrl(release, checksumAssetName()));
  const archive = await dependencies.fetchBytes(assetUrl(release, archiveName));
  verifyChecksum(archive, expectedChecksum(sums, archiveName), archiveName);

  await mkdir(storageDirectory, { recursive: true });
  const staging = await mkdtemp(join(storageDirectory, "staging-"));
  try {
    for (const [name, bytes] of Object.entries(unzipSync(archive))) {
      // archiveはzip slipを防ぐためにpath分離子を含む名前を持たない。
      if (name.includes("/") || name.includes("\\") || name === "" || name === "..") {
        throw new Error(`unexpected-archive-entry: ${name}`);
      }
      await writeFile(join(staging, name), bytes);
    }
    const staged = join(staging, executableName(dependencies.platform));
    if (!(await isFile(staged))) throw new Error("archive-executable-missing");
    if (dependencies.platform !== "win32") await chmod(staged, 0o755);
    await mkdir(dirname(directory), { recursive: true });
    await rm(directory, { force: true, recursive: true });
    await rename(staging, directory);
  } finally {
    await rm(staging, { force: true, recursive: true });
  }
  await removeOtherVersions(storageDirectory, basename(directory));
  dependencies.onProgress?.(`Verified and installed the Language Server ${release.version}.`);
  return executable;
}

/**
 * 取得した版だけを残します。
 *
 * 消さないと更新のたびに保存領域が増え続けます。削除に失敗しても取得は成功して
 * いるため、起動を止めません。
 */
async function removeOtherVersions(storageDirectory: string, keep: string): Promise<void> {
  let entries: string[];
  try {
    entries = await readdir(storageDirectory);
  } catch {
    return;
  }
  for (const entry of entries) {
    if (entry === keep || !entry.startsWith("adocweave-")) continue;
    await rm(join(storageDirectory, entry), { force: true, recursive: true }).catch(
      () => undefined,
    );
  }
}
