# AdocWeave browser v0.47.0

## 主な変更

- **配布物の形式をnpmのtarballへ変更しました。** 成果物の名前が `adocweave-browser-<version>.tar.xz` から `adocweave-browser-<version>.tgz` になり、展開後のrootが `adocweave-browser-<version>/` から `package/` になります。npm Registryへ公開する準備として、GitHub Releaseへ添付する成果物そのものをnpmの形式に揃えました。registry向けに構築し直さないため、どちらの経路でも同じbyte列を取得できます。
- **packageのREADMEをMarkdownへ変更しました。** package registryのpackageページで描画できる形式にします。記載内容は変えていません。

公開API、WebAssemblyとの通信内容および解析結果は変えていません。

## 対応環境

WebAssemblyとWeb Workerに対応したブラウザーで動作します。公開entry、WorkerおよびWASMは同一originから配信します。

## 対応関係

WebAssemblyとの通信はschema handshakeで検査します。Browser packageのバージョンを、AdocWeaveのほかの製品との互換性判断には使用しません。

## v0.47.0への移行

- GitHub Releaseのarchiveから導入している場合は、展開コマンドを `tar -xzf` へ変更してください。
- 展開後のディレクトリ名が `package` になります。`adocweave-browser-<version>` を前提にした移動やコピーの指定を変更してください。
- bundlerからの利用方法、公開API、`defaultAssetUrls` の使い方は変わりません。

## 更新とロールバック

新しいarchiveを別のディレクトリへ展開し、`worker/index.mjs` と `wasm` の相対関係を保ったまま配備先を切り替えてください。受入確認が終わるまで以前のディレクトリを保持すると、問題がある場合に元へ戻せます。

## 既知の制約

- 一つのclientは同時に一つの解析だけを実行します。並行して解析する場合はclientを分けます。
- 取消しまたはWebAssemblyのtrapが発生した場合、clientはそのWorkerを終了します。同じWorkerとWASM instanceを次の解析へ再利用しません。
- HTMLの信頼方針は利用側が決めます。packageは出力を文字列として返します。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
