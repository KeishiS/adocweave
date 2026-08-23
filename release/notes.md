# AdocWeave cli v0.47.0

## 主な変更

- **プロジェクト設定の`schema-version`を2へ更新しました。** CLIはversion 1の設定を拒否します。`workspace.scan.exclude`に同じpatternを複数回書いた場合は、一度だけ適用します。
- **`adocweave config show`が`workspace.scan.exclude`の実効値を表示するようにしました。** `.git`、`.venv`、`node_modules`および`target`の組込みpatternに、設定ファイルで指定した追加分を続けて表示します。
- **製品ごとにtagとGitHub Releaseを分けました。** CLIの成果物はCLI専用のReleaseから取得できます。
- **CLIのstable Releaseと同じcommitから構築したNix packageをCachixへ登録します。** x86_64-linuxとaarch64-linuxでは、条件が一致すればRustによる再構築を避けられます。

## 対応環境

対応targetはRelease添付の配布manifestで確認できます。

## 対応関係

CLIとLanguage Serverは同じ形式のプロジェクト設定を読み込みます。設定を共有する場合は、両方を0.47.0へ更新してください。

## v0.47.0への移行

`.adocweave.toml`の`schema-version`を2へ変更します。`workspace.scan.exclude`へ組込みpatternを再列挙している場合は削除し、リポジトリ固有の追加分だけを残してください。

変更前:

```toml
schema-version = 1

[workspace.scan]
exclude = ["**/.git", "**/.venv", "**/node_modules", "**/target", "**/generated"]
```

変更後:

```toml
schema-version = 2

[workspace.scan]
exclude = ["**/generated"]
```

`exclude = []`は追加patternがないことを表します。組込みの4 patternは解除できません。

CLIとLanguage Serverの両方で同じ`.adocweave.toml`を使用している場合は、両製品の0.47.0を導入してから、実行ファイルの切替とversion 2への設定変更を一組の操作として行います。片方だけを0.47.0へ切り替えた状態では、version 1と2のどちらか一方の設定で新旧両方を動かすことはできません。CLIだけを使用している場合は、Language Serverを導入する必要はありません。

CLIを取得する処理では、従来の`vX.Y.Z`形式ではなく`adocweave-cli/v0.47.0`を指定してください。このReleaseにはCLIの成果物だけを添付します。

配布manifestを機械処理している場合は、`schemaVersion` 5へ対応してください。最上位の`packageVersion`は`productVersion`へ変わり、`product`が加わりました。`assets`の各要素は`name`だけを持ち、従来の`kind`、`target`、`archive`、`byteSize`、`sha256`および`executable`は含みません。対象環境とarchive形式は成果物名と導入文書で確認し、SHA-256は`sha256.sum`で検査してください。`adocweave.spdx.json`はこの版から添付しません。

Nix flakeでは、`overlays.default`と`packages.${system}.adocweave`、`packages.${system}.adocweave-cli`および`packages.${system}.adocweave-lsp`を削除しました。導入するpackageは`packages.${system}.default`へ変更してください。このpackageにはCLIとLanguage Serverの両方が含まれます。CLIは`apps.${system}.default`、Language Serverは`apps.${system}.adocweave-lsp`で実行できます。

## 更新とロールバック

CLIをバージョン別directoryへ展開し、`adocweave --version --json`が0.47.0を示すことを確認してから利用先を切り替えてください。Language Serverと設定を共有している場合は、両方の実行ファイルと`.adocweave.toml`を一組で切り替えます。version 1の設定を使う旧版へ戻す場合も、使用している実行ファイル、`schema-version`および`exclude`を一組で以前の内容へ戻します。

## 既知の制約

- 組込みの`.git`、`.venv`、`node_modules`および`target`をLanguage Serverの初期走査の対象へ戻す設定はありません。
- `workspace.scan.exclude`はLanguage Serverの初期走査だけに適用します。CLIへ直接渡した入力は除外せずに読み込みます。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveと`adocweave-dist-manifest.json`を検査します。続いて、downloadした各fileに対して`gh attestation verify <asset> --repo KeishiS/adocweave`を実行し、生成元を検証してください。第三者依存の名前、versionおよびlicenseはarchive内の`THIRD_PARTY_NOTICES.adoc`で確認できます。
