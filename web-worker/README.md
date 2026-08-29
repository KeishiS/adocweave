# AdocWeave WebAssembly package

`@adocweave/wasm` は、AdocWeaveのWebAssembly moduleとWeb Worker clientを一組として提供します。

## 導入

```console
npm install @adocweave/wasm@X.Y.Z
```

GitHub Releaseへ添付したarchiveと同じbyte列をnpmへ公開します。checksumとattestationを自分で検証してから導入する場合は、AdocWeaveのGitHub Releaseに添付した `.tgz` を指定することもできます。

## 入口の選び方

用途によって2つの入口があります。要求と応答の形式はどちらも同じです。

| 入口 | 実行環境 | 取消し | WASM trapの分離 |
|---|---|---|---|
| `@adocweave/wasm` | ブラウザー | あり（`AbortSignal`） | あり（Workerを終了） |
| `@adocweave/wasm/direct` | Node | なし | なし |

ブラウザーで文書を編集しながら結果を更新する場合は、既定の入口を使います。Web Workerで実行するため、入力の続く間もUIが止まらず、前の解析を取り消せます。

静的サイト生成のようにビルド時へ組み込む場合は `direct` を使います。同じNode.jsプロセスで同期的に実行し、Web Workerを必要としません。処理を1つずつ順に実行する用途に限ります。

## ビルド時の利用

```javascript
import { analyze } from "@adocweave/wasm/direct";

const result = await analyze({
  source: { id: "docs/example.adoc", text: source },
  products: { html: true },
});
```

同梱WebAssemblyの位置はpackage内部から求めるため、pathを渡す必要はありません。初期化は最初の呼び出しで一度だけ行います。明示的に持ちたい場合は `createDirectAnalyzer()` を使います。

利用側で画像やリンク先を解決する場合は、2回に分けて実行します。1回目で問い合わせを受け取り、解決した結果を2回目へ渡します。

```javascript
import { analyze } from "@adocweave/wasm/direct";

// 1. 問い合わせと診断を受け取る
const queried = await analyze({
  source: { id: sourceId, text: source },
  products: { resourceQueries: true, diagnostics: true, document: true },
});

// 2. 利用側が解決する。存在確認、配置先の決定、公開URLの組み立ては利用側の責務
const resources = queried.resourceQueries.map((query) => resolve(query));

// 3. 解決済みresourceでHTMLへ変換する
const rendered = await analyze({
  source: { id: sourceId, text: source },
  products: {
    html: { activeUrls: { allowedSchemes: ["http", "https"] } },
  },
  resources: { assets: resources },
});
```

WASMがtrapした場合、`direct` は同じNode.jsプロセスで実行しているため以降の結果を保証しません。`createDirectAnalyzer()` で新しいanalyzerを作り直してください。

## 内容

- `wasm/adocweave_wasm.js` と `wasm/adocweave_wasm_bg.wasm` — `wasm-bindgen` のweb向け成果物
- `wasm/adocweave_wasm.d.ts` — 生成された場合のTypeScript declaration
- `worker/index.mjs` と `worker/index.d.mts` — frontend向けの公開ES module入口と型定義
- `worker/client.mjs` と `worker/worker.mjs` — 一つの要求をWeb Workerで実行する内部実装
- `worker/contracts.mjs` — packageの版情報
- `worker/protocol.d.mts` — WebAssemblyとやり取りするJSONの型。Rustのwire定義から生成します
- `worker/worker-protocol.mjs` — clientとWorkerが交換する内部封筒
- `THIRD_PARTY_NOTICES.adoc` — rootとZedのlockfileから生成したthird-party package一覧

## 最小例

```javascript
import { AdocWeaveClient, defaultAssetUrls } from "@adocweave/wasm";

const entryUrl = new URL("./worker/index.mjs", import.meta.url);
const client = new AdocWeaveClient(defaultAssetUrls(entryUrl));
const controller = new AbortController();

const result = await client.analyze({
  source: { text: "= Title\n\nAsciiDoc text" },
  products: { html: true },
}, { signal: controller.signal });

// HTMLの信頼方針はホストが決めます。この例では文字列として表示します。
preview.textContent = result.html;
```

公開する解析入口は `analyze(request, { signal })` だけです。この関数は解析結果をPromiseで返します。一つのclientは同時に一つの解析だけを実行します。異なる文書を並行して解析する場合はclientを分けます。

入力欄などの連続更新をまとめる待ち時間、文書の版および古い結果を採用するかどうかは、入力を持つ利用側アプリが管理します。利用側は処理ごとに `AbortController` を作り、前の処理が不要になったときは `abort()` を呼びます。取消しを含む全ての失敗では、`code`と英語の`message`を持つ`AdocWeaveError`でPromiseをrejectします。取消しまたはWASMのtrapが発生するとclientはそのWorkerを終了し、同じWorkerとWASM instanceを次の解析に再利用しません。

clientとWorkerの要求番号は内部実装であり、結果へ公開しません。JavaScriptとWASMの組合せはWASM protocolのschema handshakeで検査します。Worker内部の封筒やpackageのバージョンを、利用側の互換性判定には使用しません。

## 配備

`defaultAssetUrls(baseUrl)` の `baseUrl` には、配備後の公開entryである `worker/index.mjs` のURLを必ず渡します。省略した `import.meta.url` やbundle後のchunk URLを基準にしません。Worker、WASMのJavaScript補助fileおよびWASM binaryは、返されたURLで取得できる同一originへ配置します。

ViteまたはWebpackではpackageの `worker` と `wasm` を同じ公開ディレクトリへ資産としてコピーし、そのディレクトリに配備した `worker/index.mjs` のURLを `defaultAssetUrls` へ渡します。WorkerとWASMをJavaScript bundleへinline化したり、生成されたchunkの相対位置に依存したりしません。content hash付きの公開base URLを使う場合も、`worker` と `wasm` の相対関係を維持します。

静的serverは `.mjs` と `.js` を `text/javascript`、`.wasm` を `application/wasm` として返します。main document、公開entry、WorkerおよびWASMを同一originから配信します。CSPは少なくとも `script-src 'self' 'wasm-unsafe-eval'`、`worker-src 'self'` および `connect-src 'self'` を許可します。inline scriptと `unsafe-eval` は不要です。

動作する例は `example/index.html` に含まれます。Release gateでは実パッケージをesbuildでbundleしたbrowser smoke、CSP、WASM sizeおよびURL解決を検証します。

## ライセンス

MIT OR Apache-2.0
