# AdocWeave WebAssembly package

[![npm](https://img.shields.io/npm/v/%40adocweave%2Fwasm?label=npm)](https://www.npmjs.com/package/@adocweave/wasm)
[![License](https://img.shields.io/npm/l/%40adocweave%2Fwasm)](#ライセンス)

`@adocweave/wasm`は、AsciiDoc文書の解析とHTML変換をブラウザとNode.jsで利用するためのWebAssemblyライブラリです。

## インストール

```console
npm install @adocweave/wasm
```

## 入口の選択

| 入口 | 用途 | 処理の取り消し |
|---|---|---|
| `@adocweave/wasm` | ブラウザのWeb Worker | `AbortSignal`に対応 |
| `@adocweave/wasm/direct` | Node.jsでの逐次処理 | 非対応 |

ブラウザでは既定の入口を使います。Web Workerで処理するため、文書の編集中も画面操作を続けられます。
静的サイトの生成など、Node.jsで処理を一つずつ実行する場合は`direct`を使います。

## ブラウザ

```javascript
import { AdocWeaveClient, defaultAssetUrls } from "@adocweave/wasm";

const entryUrl = new URL("./worker/index.mjs", import.meta.url);
const client = new AdocWeaveClient(defaultAssetUrls(entryUrl));
const controller = new AbortController();

const result = await client.analyze({
  source: { text: "= Title\n\nAsciiDoc text" },
  products: { html: true },
}, { signal: controller.signal });

preview.textContent = result.html;
```

入力が変わり、前の処理が不要になった場合は`controller.abort()`を呼びます。異なる文書を同時に処理する場合は、文書ごとにclientを作成します。

## Node.js

```javascript
import { analyze } from "@adocweave/wasm/direct";

const result = await analyze({
  source: { id: "docs/example.adoc", text: source },
  products: { html: true },
});
```

同梱したWebAssemblyは最初の呼び出しで初期化されます。通常はWebAssemblyファイルの場所を指定する必要はありません。

## ブラウザへの配備

`worker`と`wasm`の相対位置を保ったまま、同じオリジンから配信してください。`defaultAssetUrls`には、配備した`worker/index.mjs`のURLを渡します。

`.mjs`と`.js`は`text/javascript`、`.wasm`は`application/wasm`として配信します。CSPでは、少なくとも`script-src 'self' 'wasm-unsafe-eval'`、`worker-src 'self'`および`connect-src 'self'`を許可してください。

詳しい配備方法は[WebAssemblyライブラリの配備](https://github.com/KeishiS/adocweave/blob/main/docs/user-guide/release-installation.adoc#wasm-library-deployment)、APIは[WebAssembly API](https://github.com/KeishiS/adocweave/blob/main/docs/developer-guide/core-profile.adoc#wasm-api)を参照してください。

## ライセンス

[MIT License](LICENSE-MIT)または[Apache License 2.0](LICENSE-APACHE)を選択できます。
