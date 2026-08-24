# AdocWeave wasm v0.48.0

## 主な変更

- **package名を `@adocweave/browser` から `@adocweave/wasm` へ変更しました。** WebAssemblyは静的サイト生成のようにNodeのビルド時でも動きます。動作環境を名前で限定しないよう、Rust crateと同じ `wasm` へ揃えました。`@adocweave/browser` は非推奨とし、移動先を示します。
- **成果物とtagの名前を変更しました。** 成果物は `adocweave-wasm-<version>.tgz`、tagは `adocweave-wasm/v<version>` になります。
- **公開定数 `BROWSER_PACKAGE_VERSION` を `WASM_PACKAGE_VERSION` へ改名しました。**

`AdocWeaveClient`、`defaultAssetUrls`、`analyze` の要求と応答、WebAssemblyとの通信内容および解析結果は変えていません。

## 対応環境

WebAssemblyとWeb Workerに対応したブラウザーで動作します。公開entry、WorkerおよびWASMは同一originから配信します。

## 対応関係

WebAssemblyとの通信はschema handshakeで検査します。packageのバージョンを、AdocWeaveのほかの製品との互換性判断には使用しません。

## v0.48.0への移行

- 依存の指定を `@adocweave/wasm` へ変更してください。`@adocweave/browser` へは新しい版を公開しません。
- `BROWSER_PACKAGE_VERSION` を参照している場合は `WASM_PACKAGE_VERSION` へ変更してください。
- GitHub Releaseのarchiveから導入している場合は、成果物名を `adocweave-wasm-<version>.tgz` へ変更してください。展開後のrootは `package` のままです。
- `AdocWeaveClient` の使い方は変わりません。

## 更新とロールバック

npmから導入している場合は、指定するバージョンを変更して `npm install` を実行し、`package-lock.json` の差分を確認してください。archiveから導入している場合は、新しいarchiveを別のディレクトリへ展開し、`worker/index.mjs` と `wasm` の相対関係を保ったまま配備先を切り替えます。受入確認が終わるまで以前の状態を保持すると、問題がある場合に元へ戻せます。

## 既知の制約

- 一つのclientは同時に一つの解析だけを実行します。並行して解析する場合はclientを分けます。
- 取消しまたはWebAssemblyのtrapが発生した場合、clientはそのWorkerを終了します。同じWorkerとWASM instanceを次の解析へ再利用しません。
- HTMLの信頼方針は利用側が決めます。packageは出力を文字列として返します。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
