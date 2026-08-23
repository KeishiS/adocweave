# AdocWeave lsp v0.47.0

## 主な変更

- **`workspace.scan.exclude`を組込みの除外patternへ追加する設定に変更しました。** `.git`、`.venv`、`node_modules`および`target`は常に初期走査から除外し、設定ファイルにはリポジトリ固有の追加分だけを記載できます。
- **プロジェクト設定の`schema-version`を2へ更新しました。** CLIとLanguage Serverはversion 1の設定を拒否し、同じpatternが重複した場合は一度だけ適用します。
- **不完全なワークスペース走査の警告を、発生期間ごとに管理するようにしました。** 同じ原因は不完全な状態が続く間に一度だけ通知します。不完全な状態が続いていても新しい原因が加わった場合は再度通知し、複数の上限へ同時に到達した場合は原因をまとめます。完全な走査へ回復した後やworkspace folderの変更後に再発した場合も再度通知します。

## 対応環境

対応targetはRelease添付の配布manifestで確認できます。

## 対応関係

エディターとの機能の対応は、標準のLanguage Server Protocol capabilityで確認してください。

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

`exclude = []`は追加patternがないことを表します。組込みの4 patternは解除できないため、version 1で空の配列を使っていた場合も初期走査の対象には戻りません。

CLIとLanguage Serverの両方で同じ`.adocweave.toml`を使用している場合は、両製品の0.47.0が公開されるまで設定を変更しないでください。両方を展開してから、実行ファイルの切替とversion 2への設定変更を一組の操作として行います。片方だけを0.47.0へ切り替えた状態では、version 1と2のどちらか一方の設定で新旧両方を動かすことはできません。Language Serverだけを使用している場合は、CLIを導入する必要はありません。

この版から製品ごとにtagとGitHub Releaseを分けます。Language Serverを取得する処理では、従来の`vX.Y.Z`形式ではなく`adocweave-lsp/v0.47.0`を指定してください。このReleaseにはLanguage Serverの成果物だけを添付します。

配布manifestを機械処理している場合は、`schemaVersion` 5へ対応してください。最上位の`packageVersion`は`productVersion`へ変わり、`product`が加わりました。`assets`の各要素は`name`だけを持ち、従来の`kind`、`target`、`archive`、`byteSize`、`sha256`および`executable`は含みません。対象環境とarchive形式は成果物名と導入文書で確認し、SHA-256は`sha256.sum`で検査してください。`adocweave.spdx.json`はこの版から添付しません。

Nix flakeでは、`overlays.default`と`packages.${system}.adocweave`、`packages.${system}.adocweave-cli`および`packages.${system}.adocweave-lsp`を削除しました。導入するpackageは`packages.${system}.default`へ変更してください。このpackageにはCLIとLanguage Serverの両方が含まれ、Language Serverの実行には引き続き`apps.${system}.adocweave-lsp`を使用できます。

## 更新とロールバック

Language Serverをバージョン別directoryへ展開し、`adocweave-lsp --version --json`が0.47.0を示すことを確認してから利用先を切り替えてください。CLIと設定を共有している場合は、両方の実行ファイルと`.adocweave.toml`を一組で切り替えます。version 1の設定を使う旧版へ戻す場合も、使用している実行ファイル、`schema-version`および`exclude`を一組で以前の内容へ戻します。

## 既知の制約

- 組込みの`.git`、`.venv`、`node_modules`および`target`を初期走査の対象へ戻す設定はありません。
- `workspace.scan.exclude`は初期走査だけに適用します。明示的に開いた文書、file watcherの既知の通知、include先およびCLIへ直接渡した入力は読み込めます。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveと`adocweave-dist-manifest.json`を検査します。続いて、downloadした各fileに対して`gh attestation verify <asset> --repo KeishiS/adocweave`を実行し、生成元を検証してください。第三者依存の名前、versionおよびlicenseはarchive内の`THIRD_PARTY_NOTICES.adoc`で確認できます。
