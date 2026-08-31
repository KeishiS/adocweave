# AdocWeave textlint AsciiDoc plugin

`@adocweave/textlint-plugin-asciidoc` は、AsciiDoc文書をAdocWeaveで解析し、textlintが扱うTxtASTへ変換するProcessor Pluginです。Node.js、textlintおよびこのパッケージだけで動作し、Rust、Cargoまたは実行時の追加ダウンロードを必要としません。

## 導入

```console
npm install --save-dev textlint @adocweave/textlint-plugin-asciidoc
```

バージョンを固定する場合は、使用するパッケージの `X.Y.Z` を指定します。このパッケージのバージョンは
ネイティブ実行ファイルやほかのnpmパッケージとは独立しています。

```console
npm install --save-dev textlint@15.8.0 \
  @adocweave/textlint-plugin-asciidoc@X.Y.Z
```

プロジェクトへ依存を導入せずに一度だけ試す場合は、`npx` の `--package` でtextlint、Processorおよび規則を一時的に取得します。次の例は日本語技術文書向けの規則で `document.adoc` を検査します。

```console
npx --yes \
  --package=textlint@15.8.0 \
  --package=@adocweave/textlint-plugin-asciidoc@X.Y.Z \
  --package=textlint-rule-preset-ja-technical-writing@12.0.2 \
  textlint --no-textlintrc \
  --plugin @adocweave/asciidoc \
  --preset ja-technical-writing \
  document.adoc
```

この方法では作業ディレクトリへ `package.json`、`package-lock.json` および `node_modules` を作成しません。取得したパッケージはnpmのキャッシュへ保存されます。継続して使う場合は、前述の方法で依存とバージョンをプロジェクトへ記録してください。

## 設定

`.textlintrc.json` では、利用する日本語規則などとは別にProcessorを指定します。

```json
{
  "plugins": ["@adocweave/asciidoc"],
  "rules": {
    "your-rule": true
  }
}
```

## 対応範囲

既定では `.adoc`、`.asciidoc` および `.asc` を扱います。別の拡張子を扱う場合は、先頭に `.` を付けて追加します。

```json
{
  "plugins": {
    "@adocweave/asciidoc": {
      "extensions": [".guide"]
    }
  }
}
```

見出し、block title、段落、リスト、引用、表、リンク、footnote本文、画像の代替文、iconの代替文またはtitle、audioとvideoのtitleおよびインライン装飾を文章として渡します。TxtASTにはblock title専用の標準nodeがないため、block titleはdepth 1の `Header` としてtitle向け規則の対象にします。

コード、UI macroの表示要素、表示文字列のないURL、数式、属性参照、passおよび未対応構文は `Code` として構造だけを保ち、`Str` を対象とする文章規則から除外します。macroの文章部分に属性参照またはinline記法がある場合も、誤った文字列を渡さないためmacro全体を除外します。titleなどがないmedia macroのファイル名も文章として扱いません。includeは展開せず、入力された一つの物理ファイルだけを解析します。

## 自動修正

このProcessorは自動修正に対応しません。規則が修正情報を返した場合も削除するため、`textlint --fix` でAsciiDoc文書を書き換えません。検出した問題は元の文書上の行と列へ報告します。

## 使用量の上限

一つの入力は10 MiB、TxtAST planは50 MiB、planのnodeは1,000,000件、`sourceId` は4 KiBを上限とします。同梱WebAssemblyのlinear memoryは256 MiBを上限とし、完成した配布物で上限が埋め込まれていることを検査します。上限を超えた場合は、`code` を持つ `Error` として処理を中止します。配布するtarballは8 MiB、展開後は16 MiBを上限とします。収録ファイルは `package.json` の `files` で指定します。

## 対応環境

Node.js 24.19.0とtextlint 15.8.0を検証対象とします。CIではこの固定した組合せで導入と実行を確認します。WebAssemblyはパッケージへ同梱し、別のAdocWeaveバージョンへ差し替える機能は提供しません。

## ライセンス

MIT OR Apache-2.0
