# AdocWeave zed v0.47.0

## 主な変更

- **Language Serverを自動で取得します。** これまでは拡張を導入しても、別途`adocweave-lsp`を導入して`PATH`または設定へ登録するまで文書解析が動きませんでした。どちらにも見つからない場合、拡張がGitHub Releaseから最新のLanguage Serverを取得して起動します。
- **探索順は設定の絶対path、`PATH`、自動取得の三段です。** 導入済みの実行ファイルがあれば常にそちらが選ばれ、自動取得は行われません。
- **自動取得にはchecksumとattestationの検証がありません。** Zedの拡張APIに検証手段がないためです。検証したうえで導入したい場合は、これまでどおり`adocweave-lsp`を自分で導入し、`lsp.adocweave.binary.path`または`PATH`で指定してください。

## 対応環境

自動取得はLinuxのx86-64とARM64、macOSのARM64、Windowsのx86-64に対応します。Intel macOSとWindows ARM64向けのnative archiveは配布していないため、これらの環境では自動取得へ進まず、対応環境を示して起動に失敗します。手動で導入した実行ファイルの指定は、この制限を受けません。

Zed Extension Galleryへ公開していないため、展開したディレクトリをdev extensionとして導入します。

## 対応関係

拡張とLanguage Serverの製品バージョンを一致させる必要はありません。利用できる機能は、接続時に交換する標準LSP capabilityから決まります。自動取得では、公開済みの`adocweave-lsp`のうち最新の安定版を選びます。

## v0.47.0への移行

- 利用者の作業は不要です。すでに`adocweave-lsp`を導入している場合、その実行ファイルが引き続き優先されます。
- 自動取得を使いたくない場合は、`adocweave-lsp`を導入して`lsp.adocweave.binary.path`または`PATH`で指定してください。拡張側に自動取得を止める設定は設けていません。

## 更新とロールバック

新しいバージョンのZedディレクトリを別に展開し、dev extensionをそのディレクトリへ再設定します。以前の版へ戻す場合は、同じ手順で古いディレクトリを指定し直します。旧ディレクトリは、新しい版で編集機能を確認するまで保持してください。

自動取得したLanguage Serverは、Zedが拡張へ割り当てるworkingディレクトリの`adocweave-lsp-<version>`へ置きます。拡張は取得した版だけを残し、以前の版を削除します。取得する版を固定したい場合は、`lsp.adocweave.binary.path`または`PATH`で指定してください。

## 既知の制約

- 自動取得の完全性はTLSだけに依存します。checksumとattestationは検証しません。
- 自動取得するLanguage Serverの版を拡張の設定から固定できません。常に最新の安定版を取得します。
- Zed Extension Galleryへ公開していないため、dev extensionとして導入します。拡張自体の更新は自動化されません。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
