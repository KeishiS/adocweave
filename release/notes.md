# AdocWeave lsp v0.47.0

## 主な変更

- **`workspace.scan.exclude`を組込みの除外patternへ追加する設定に変更しました。** `.git`、`.venv`、`node_modules`および`target`は常に初期走査から除外し、設定ファイルにはリポジトリ固有の追加分だけを記載できます。
- **プロジェクト設定の`schema-version`を2へ更新しました。** CLIとLanguage Serverはversion 1の設定を拒否し、同じpatternが重複した場合は一度だけ適用します。

## 対応環境

Linux、macOSおよびWindows向けのLanguage Serverを配布します。対応するOSとCPUは配布manifestで確認できます。

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

CLIとLanguage Serverは同じ`.adocweave.toml`を使用し、0.47.0ではどちらもversion 1を拒否します。両製品の0.47.0が公開されるまで設定を変更せず、両方を展開してから、実行ファイルの切替とversion 2への設定変更を一組の操作として行ってください。片方だけを0.47.0へ切り替えた状態では、version 1と2のどちらか一方の設定で新旧両方を動かすことはできません。

## 更新とロールバック

CLIとLanguage Serverをそれぞれバージョン別directoryへ展開し、両方の`--version --json`が0.47.0を示すことを確認してから、利用先と設定を一組で切り替えてください。version 1の設定を使う旧版へ戻す場合も、両方の実行ファイル、`.adocweave.toml`の`schema-version`および`exclude`を一組で以前の内容へ戻します。

## 既知の制約

- 組込みの`.git`、`.venv`、`node_modules`および`target`を初期走査の対象へ戻す設定はありません。
- `workspace.scan.exclude`は初期走査だけに適用します。明示的に開いた文書、file watcherの既知の通知、include先およびCLIへ直接渡した入力は読み込めます。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
