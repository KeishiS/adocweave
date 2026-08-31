# AdocWeave textlint AsciiDoc plugin

[![npm](https://img.shields.io/npm/v/%40adocweave%2Ftextlint-plugin-asciidoc?label=npm)](https://www.npmjs.com/package/@adocweave/textlint-plugin-asciidoc)
[![License](https://img.shields.io/npm/l/%40adocweave%2Ftextlint-plugin-asciidoc)](#ライセンス)

`@adocweave/textlint-plugin-asciidoc`は、AsciiDoc文書をtextlintで検査するためのProcessor Pluginです。WebAssemblyを同梱しているため、RustやAdocWeaveの実行ファイルは必要ありません。

## インストール

```console
npm install --save-dev textlint @adocweave/textlint-plugin-asciidoc
```

## 設定

`.textlintrc.json`へProcessorと使用する規則を指定します。

```json
{
  "plugins": ["@adocweave/asciidoc"],
  "rules": {
    "your-rule": true
  }
}
```

次のコマンドでAsciiDoc文書を検査できます。

```console
npx textlint document.adoc
```

## 対応範囲

既定では`.adoc`、`.asciidoc`および`.asc`を扱います。別の拡張子を追加する場合は、先頭に`.`を付けて指定します。

```json
{
  "plugins": {
    "@adocweave/asciidoc": {
      "extensions": [".guide"]
    }
  }
}
```

見出し、段落、リスト、引用、表、リンク、脚注、画像の代替文などを文章として規則へ渡します。コード、URL、数式、属性参照および`pass`は文章の検査対象から外します。

`include`は展開せず、指定されたファイルだけを検査します。自動修正には対応していないため、`textlint --fix`を実行してもAsciiDoc文書を書き換えません。

Node.jsとtextlintの対応バージョンは[package.json](https://github.com/KeishiS/adocweave/blob/main/packages/textlint-plugin-asciidoc/package.json)を参照してください。変更内容は[CHANGELOG](https://github.com/KeishiS/adocweave/blob/main/packages/textlint-plugin-asciidoc/CHANGELOG.md)に記載します。

## ライセンス

[MIT License](LICENSE-MIT)または[Apache License 2.0](LICENSE-APACHE)を選択できます。
