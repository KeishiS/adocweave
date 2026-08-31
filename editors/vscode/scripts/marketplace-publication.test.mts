import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { strToU8, zipSync, type Zippable } from "fflate";

import { checkMarketplacePublication } from "./marketplace-publication.mts";

const expected = {
  name: "adocweave",
  publisher: "adocweave",
  target: "universal",
  version: "1.2.3",
};

function packageEntries(contents = "extension"): Zippable {
  return {
    "[Content_Types].xml": strToU8(`<?xml version="1.0"?>
<Types><Default Extension="json" ContentType="application/json"/>
<Override PartName="/extension.vsixmanifest" ContentType="text/xml"/></Types>`),
    "extension.vsixmanifest": strToU8(
      `<PackageManifest><Metadata><Identity Id="${expected.name}" Publisher="${expected.publisher}" Version="${expected.version}"/></Metadata></PackageManifest>`,
    ),
    "extension/package.json": strToU8(JSON.stringify(expected)),
    "extension/dist/extension.cjs": strToU8(contents),
  };
}

function signedEntries(contents = "extension"): Zippable {
  return {
    ...packageEntries(contents),
    "[Content_Types].xml": strToU8(`<?xml version="1.0"?>
<Types><Default ContentType="application/json" Extension="json"/>
<Default Extension="sigs" ContentType="application/vnd.openxmlformats-package.digital-signature-origin"/>
<Override ContentType="text/xml" PartName="/extension.vsixmanifest"/>
<Override PartName="/_xmlsignatures/sig1.xml" ContentType="application/vnd.openxmlformats-package.digital-signature-xmlsignature+xml"/></Types>`),
    "_rels/.rels": strToU8(
      `<Relationships><Relationship Id="rId1" Target="_xmlsignatures/origin.sigs" Type="http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/origin"/></Relationships>`,
    ),
    "_xmlsignatures/origin.sigs": new Uint8Array(),
    "_xmlsignatures/_rels/origin.sigs.rels": strToU8("signature relationship"),
    "_xmlsignatures/sig1.xml": strToU8("signature"),
  };
}

async function fixture(
  candidateEntries = packageEntries(),
): Promise<{ candidatePath: string; outputPath: string; close: () => Promise<void> }> {
  const directory = await mkdtemp(join(tmpdir(), "adocweave-marketplace-"));
  const candidatePath = join(directory, "extension.vsix");
  await writeFile(candidatePath, zipSync(candidateEntries));
  return {
    candidatePath,
    outputPath: join(directory, "published.vsix"),
    close: () => rm(directory, { force: true, recursive: true }),
  };
}

function response(entries: Zippable, status = 200): Response {
  return new Response(zipSync(entries), { status });
}

test("未公開のversionをmissingとして報告する", async () => {
  const source = await fixture();
  try {
    let requested = "";
    assert.equal(
      await checkMarketplacePublication({
        ...expected,
        candidatePath: source.candidatePath,
        request: async (input) => {
          requested = String(input);
          return new Response(null, { status: 404 });
        },
      }),
      "missing",
    );
    assert.equal(
      requested,
      "https://marketplace.visualstudio.com/_apis/public/gallery/publishers/adocweave/vsextensions/adocweave/1.2.3/vspackage",
    );
  } finally {
    await source.close();
  }
});

test("Marketplace署名を除く内容が同じならpublishedとして報告する", async () => {
  const source = await fixture();
  try {
    const published = zipSync(signedEntries());
    assert.equal(
      await checkMarketplacePublication({
        ...expected,
        candidatePath: source.candidatePath,
        outputPath: source.outputPath,
        request: async () => new Response(published),
      }),
      "published",
    );
    assert.deepEqual(await readFile(source.outputPath), Buffer.from(published));
  } finally {
    await source.close();
  }
});

test("公開直後にVSIXが準備中ならpendingとして報告する", async () => {
  const source = await fixture();
  try {
    assert.equal(
      await checkMarketplacePublication({
        ...expected,
        candidatePath: source.candidatePath,
        request: async () => new Response(null, { status: 202 }),
      }),
      "pending",
    );
  } finally {
    await source.close();
  }
});

test("同じversionの拡張内容が異なる場合は拒否する", async () => {
  const source = await fixture();
  try {
    await assert.rejects(
      checkMarketplacePublication({
        ...expected,
        candidatePath: source.candidatePath,
        request: async () => response(signedEntries("different")),
      }),
      /different content/u,
    );
  } finally {
    await source.close();
  }
});

test("異なるidentity、versionまたはtargetを拒否する", async () => {
  const source = await fixture();
  try {
    for (const changed of [{ publisher: "other" }, { version: "9.9.9" }, { target: "linux-x64" }]) {
      await assert.rejects(
        checkMarketplacePublication({
          ...expected,
          ...changed,
          candidatePath: source.candidatePath,
          request: async () => response(signedEntries()),
        }),
        /different extension identity, version, or target/u,
      );
    }
  } finally {
    await source.close();
  }
});

test("署名領域以外の追加fileとmetadata変更を拒否する", async () => {
  const source = await fixture();
  try {
    const extra = { ...signedEntries(), "extension/extra.txt": strToU8("extra") };
    await assert.rejects(
      checkMarketplacePublication({
        ...expected,
        candidatePath: source.candidatePath,
        request: async () => response(extra),
      }),
      /different non-signature file list/u,
    );

    const changedTypes = signedEntries();
    changedTypes["[Content_Types].xml"] = strToU8(
      `<Types><Default Extension="json" ContentType="text/plain"/></Types>`,
    );
    await assert.rejects(
      checkMarketplacePublication({
        ...expected,
        candidatePath: source.candidatePath,
        request: async () => response(changedTypes),
      }),
      /different non-signature content types/u,
    );
  } finally {
    await source.close();
  }
});
