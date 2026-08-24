# AdocWeave lib v0.47.0

## 主な変更

- **Rustライブラリを版付きの製品として公開します。** これまでworkspace versionはどの製品にも属さず、公開APIを削除しても版が変わりませんでした。今後は`adocweave-lib/v<version>`のtagとGitHub Releaseを持ちます。利用側はcommit SHAではなくtagで固定できます。
- **`output::projection`から`DocumentProjection`と`project`を削除しました。** 用途別のquery関数へ分割しています。`block_presentations`、`document_title`、`external_links`、`formulas`、`ordered_lists`、`reference_edges`、`rendering_features`および`source_blocks`が代わりです。
- **`output::conformance`から`DocumentProducts`、`ProductSet`および`products`を削除しました。** 適合性検査の内部構造であり、公開範囲から外します。
- **`output::canonical`を追加しました。** `canonical_ast`と`canonical_syntax`を公開します。

この版に含まれるcrateは`adocweave`、`adocweave-config`、`adocweave-host`、`adocweave-textlint`および`adocweave-workspace`です。crates.ioへは公開しません。

## 対応環境

Rust 1.97.1で構築します。対応環境はRustのtoolchainに従います。

## 対応関係

CLI、Language Server、WebAssemblyおよびtextlint用Processorは、それぞれ独立した製品バージョンを持ちます。ライブラリの版をこれらとの互換性判断には使用しません。

## v0.47.0への移行

- gitの依存を`tag = "adocweave-lib/v0.47.0"`で固定できます。commit SHAでの固定も引き続き使えます。
- `DocumentProjection`と`project`を使っていた場合は、必要な情報に対応するquery関数へ置き換えてください。文書全体を一度に組み立てる代わりに、使う値だけを取り出します。
- `DocumentProducts`、`ProductSet`および`products`を使っていた場合は、適合性検査の外では`output::canonical`の関数を使ってください。

## 更新とロールバック

利用側の`Cargo.toml`で固定するtagを変更し、`cargo update -p adocweave`で解決し直してください。以前の版へ戻す場合は同じ手順でtagを戻します。`Cargo.lock`の差分で、解決したcommitを確認できます。

## 既知の制約

- crates.ioへ公開しません。gitの依存として取得します。
- 5つのcrateが同じ版を共有します。いずれかの変更で全体の版が上がります。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
