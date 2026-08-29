# AdocWeave vscode v0.48.0

## 主な変更

- **Language Serverを自動で取得します。** これまでは拡張を導入しても、別途`adocweave-lsp`を導入して`PATH`または設定へ登録するまで文書解析が動きませんでした。どちらにも見つからない場合、拡張がGitHub Releaseから最新のLanguage Serverを取得して起動します。
- **探索順は`adocweave.server.path`の絶対path、`PATH`、自動取得の三段です。** 導入済みの実行ファイルがあれば常にそちらが選ばれ、自動取得は行われません。
- **取得したarchiveはchecksumを照合してから展開します。** 同じReleaseの`sha256.sum`に記載された値と突き合わせ、一致しない場合は展開せず、理由を示して起動を中止します。

attestationの検証は行いません。sigstoreの検証処理を拡張へ組み込む必要があり、checksumの照合に比べて費用が大きいためです。より強い保証が必要な場合は、利用者向けの配布物導入手順、Nixまたはpackage registryから導入し、その実行ファイルを`adocweave.server.path`または`PATH`で指定してください。

## 対応環境

Visual Studio Code 1.107.0以降が必要です。

自動取得はlinuxのx86-64とARM64、macOSのARM64、Windowsのx86-64に対応します。Intel macOSとWindows ARM64向けのarchiveは配布していないため、これらの環境では自動取得へ進まず、対応環境を示して起動に失敗します。手動で導入した実行ファイルの指定は、この制限を受けません。

未信頼ワークスペースでは拡張を起動しないため、自動取得も行いません。

## 対応関係

拡張とLanguage Serverの製品バージョンを一致させる必要はありません。利用できる機能は、接続時に交換する標準LSP capabilityから決まります。自動取得では、公開済みの`adocweave-lsp`のうち最新の安定版を選びます。

## v0.48.0への移行

- 利用者の作業は不要です。すでに`adocweave-lsp`を導入している場合、その実行ファイルが引き続き優先されます。
- 自動取得を使いたくない場合は、`adocweave-lsp`を導入して`adocweave.server.path`または`PATH`で指定してください。拡張側に自動取得を止める設定は設けていません。

## 更新とロールバック

Visual Studio MarketplaceまたはOpen VSX Registryから導入した場合は、editorの拡張画面から更新および以前の版への切り替えができます。VSIXを手動で導入した場合は、新しいVSIXを`code --install-extension <file> --force`で導入し、Windowを再読込します。

自動取得したLanguage Serverは、拡張のglobal storageにある`servers`内の`adocweave-lsp-<version>-<target>`形式のディレクトリへ置きます。同じ版を取得済みの場合は再取得せず、新しい版を取得したときは以前の版を削除します。取得する版を固定したい場合は、`adocweave.server.path`または`PATH`で指定してください。

## 既知の制約

- 自動取得したLanguage Serverの版を拡張の設定から固定できません。常に最新の安定版を取得します。
- 自動取得ではattestationを検証しません。
- Marketplaceの索引更新には遅れがあり、公開直後は拡張画面へ現れないことがあります。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
