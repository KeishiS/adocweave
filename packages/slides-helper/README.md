# @adocweave/slides-helper

AdocWeaveのHTMLスライド向けに、TeX数式とCSL形式の引用・参考文献を事前処理するNode.jsパッケージです。
Node.js 24.19.0以降を使用します。実行入口は`adocweave-slides-helper`です。
起動時にパッケージの`engines.node`を検査し、必要なバージョンを満たさない場合は、
検出したバージョンと導入案内を標準エラーへ表示して終了します。

## 導入

npm Registryの配布版は、版を指定して導入します。

```console
npm install --global --ignore-scripts @adocweave/slides-helper@X.Y.Z
```

npm archiveには監査した実行時の依存一式を含め、導入時に推移依存の版を選び直しません。
独自に原稿のディレクトリへ置いた実行ファイルは自動探索しません。
AdocWeave CLIから使う場合は`PATH`へ導入するか、`--slides-helper`に入口を指定します。

ソースから使う場合は、リポジトリ直下で固定依存を導入します。

```console
npm ci --ignore-scripts --prefix packages/slides-helper
adocweave convert talk.adoc --to revealjs --output dist/talk \
  --slides-helper /absolute/path/to/adocweave/packages/slides-helper/bin.mjs
```

通常の本文と手書きの書誌定義だけの生成には、この補助パッケージは必要ありません。
数式やCSL引用を含むスライドの記法とデータ指定は
[CLI利用手順](https://github.com/KeishiS/adocweave/blob/main/docs/user-guide/command-line.adoc#revealjs-slides)を参照してください。

## 実行

```console
adocweave-slides-helper < request.json > result.json
```

npmの実行用リンクを使わず、`node`にインストール先の`bin.mjs`を渡して直接起動することもできます。
Windowsでもこの方法を使えます。

一つのUTF-8 JSON要求を標準入力から読み、一つのJSON応答を標準出力へ書いて終了します。
診断は応答の`diagnostics`に英語で含めます。errorがある場合は終了コード1、それ以外は0です。
ライブラリのログを標準出力へ混ぜません。

## 入出力

[protocol.d.mts](./protocol.d.mts)が入出力の型定義です。空の要求も処理できます。

```json
{
  "schemaVersion": 1,
  "eqnums": "none",
  "scopes": {
    "body": { "equations": [], "citations": [] },
    "notes": { "equations": [], "citations": [] }
  }
}
```

`body`は本文、`notes`は発表者ノートの掲載範囲です。この二つだけを受け付けます。
式番号、式のlabel、TeX macroの状態、引用の状態は、掲載範囲ごとに独立します。
式と引用の配列は、それぞれ文書中の順序で渡してください。

`key`は呼出し側が生成し、同じ掲載範囲の式・引用を通じて一意にします。
先頭をASCII英字とし、英数字、`_`、`-`からなる64文字以内の識別子を使います。
応答は入力順を維持し、各keyに`status: "ok"`または`"failed"`を返します。
`failed`には同じ掲載範囲・keyのerror診断を必ず含めます。
呼出し側は不明なkey、結果の欠落・重複、失敗に対応しない診断を検査してください。
要求自体の不正と出力上限の失敗では、空の二掲載範囲と全体のerror診断を返します。
原稿のファイル名や位置は呼出し側で保持します。

`notices.math`と`notices.citations`には、使用した数式・引用の配布通知本文を返します。
それぞれ式・引用の入力がなければ`null`です。数式の通知にはフォントの著作権表示、GUST Font License、LPPL、
MathJaxのlicense、引用の通知にはciteprocの帰属表示とCPAL本文を含めます。パッケージ自身の固定同梱ファイルだけを読みます。
呼出し側は各本文64 KiB・合計256 KiB以内のUTF-8文字列として検査し、固定filenameのテキスト資産へ保存してください。
要求全体の失敗では両方`null`です。公開用の生成では、発表者ノートだけが使う通知も除去してください。

## 数式

各式は`{ key, tex, display }`で指定します。`eqnums`は要求全体に共通で、`none`、`ams`、`all`を受け付けます。
番号を自動付与するかどうかはMathJaxの規則に従います。両掲載範囲の番号は別々に1から始まります。
成功時には、ブラウザーでMathJaxを実行する必要のない静的`svg`を返します。
SVGのglyph pathを各式に含め、IDと内部リンクに掲載範囲・keyを反映します。

MathJaxの`base`、`ams`、`newcommand`、`configmacros`を使用します。
`label`、前方の`ref`・`eqref`、`tag`、`notag`、AMS環境、macro定義を処理します。
未定義のmacro、重複label、番号のある参照先がない式参照はerrorです。
TeXの`require`、拡張機能の自動取得、任意HTMLの埋込みは受け付けません。
MathJax自身のmacro展開上限とbuffer上限を有効にしています。

共通の`macros`には`{ name, definition, arguments?, default? }`を渡せます。
名前はASCII英字1〜64文字、引数は0〜9個です。`default`は最初の省略可能な引数の既定値です。
式中で定義したmacroは、同じ掲載範囲の後続の式へ引き継ぎます。

## 引用と参考文献

引用には`{ key, items }`を指定し、各itemに書誌データの`id`を渡します。
`locator`、`label`、`prefix`、`suffix`、`suppressAuthor`、`authorOnly`も指定できます。
共通の`csl`にはCSL-JSONの`items`、CSL styleのXML文字列、localeのXML文字列を渡します。
ファイル名やURLを指定して取得する操作はありません。

citeprocで掲載範囲全体を文書順に処理します。後の引用によって名前や年の区別が変わる場合は、
前の引用の表示も更新して返します。参考文献はCSL styleの順序で、引用されたitemだけを返します。
参考文献の配置は呼出し側が行います。

引用・参考文献は有限のinline treeです。文字列の`text`と、子要素を持つ`emphasis`、`strong`、
`superscript`、`subscript`、`smallcaps`、`underline`、`normal-emphasis`、`normal-strong`、
`normal-smallcaps`、`link`を使います。`normal-*`は外側から引き継いだ同種の装飾を解除します。
リンクは資格情報のないHTTP・HTTPSだけを受け付けます。
任意HTML、未対応の装飾、複数itemを一つにまとめる参考文献項目はerrorです。
citeprocが生成する既知の参考文献wrapperを除去し、番号と本文の間の区切りを保持します。

## 上限

| 対象 | 上限 |
| --- | ---: |
| 標準入力 | 4 MiB |
| JSON応答 | 32 MiB |
| 式・引用 | それぞれ両掲載範囲の合計1024件 |
| 式のTeX | 16 KiB |
| 引用一つのitem | 64件 |
| 書誌item | 4096件 |
| 共通macro | 64個、定義一つ4 KiB |
| CSL style・locale | それぞれ512 KiB |
| 通知本文 | 一つ64 KiB、合計256 KiB |

MathJaxのbuffer上限など、使用するライブラリの上限も適用されます。
AsciiDocの解析、include、原稿の取得、サーバー、監視は呼出し側の責務です。

## 配布条件

パッケージ自身はMITまたはApache-2.0で使用できます。実行時の依存ライブラリとフォントには、
それぞれの配布条件が適用されます。[THIRD_PARTY_NOTICES.adoc](./THIRD_PARTY_NOTICES.adoc)と`licenses/`に
実際に配布されたライセンス本文、著作権表示、citeprocのCPAL選択と帰属表示を収録しています。
