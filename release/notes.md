# AdocWeave vscode v0.48.1

## 主な変更

- **0.48.0で自動取得が働かなかった問題を直しました。** `adocweave-lsp`を導入していない環境で、拡張が取得へ進まず`language-server-not-found`で停止していました。取得先を決める条件が実際の環境で偽になり、自動取得が黙って無効になっていたためです。条件を外し、取得先は常に拡張のglobal storageとします。
- **選択の経過を出力パネルへ記録します。** 設定にも`PATH`にも見つからず取得へ進んだこと、および取得先を`AdocWeave`の出力に残します。起動しなかったときに、設定、`PATH`、自動取得のどこで止まったのかを切り分けられます。

この版から、自動取得は実際の環境で検証します。設定と`PATH`のどちらにもLanguage Serverがない状態でVisual Studio Codeを起動し、公開済みのGitHub Releaseから取得して診断が返るまでを、公開前の検査に含めます。0.48.0では取得経路を差し替えた単体試験しかなく、実際のnetworkとfilesystemを通る経路が検証されていませんでした。

## 対応環境

Visual Studio Code 1.107.0以降が必要です。

自動取得はlinuxのx86-64とARM64、macOSのARM64、Windowsのx86-64に対応します。これら以外では自動取得へ進まず、対応環境を示して起動に失敗します。手動で導入した実行ファイルの指定は、この制限を受けません。

未信頼ワークスペースでは拡張を起動しないため、自動取得も行いません。

## 対応関係

拡張とLanguage Serverの製品バージョンを一致させる必要はありません。利用できる機能は、接続時に交換する標準LSP capabilityから決まります。自動取得では、公開済みの`adocweave-lsp`のうち最新の安定版を選びます。

## v0.48.1への移行

- 利用者の作業は不要です。拡張画面から更新してください。
- 0.48.0で自動取得が働かなかった場合は、この版で解決します。解決しない場合は、出力パネルの`AdocWeave`に記録された行を添えて報告してください。どの段で止まったのかが分かります。

## 更新とロールバック

Visual Studio MarketplaceまたはOpen VSX Registryから導入した場合は、editorの拡張画面から更新および以前の版への切り替えができます。VSIXを手動で導入した場合は、新しいVSIXを`code --install-extension <file> --force`で導入し、Windowを再読込します。

自動取得したLanguage Serverは、拡張のglobal storageにある`servers`内の`adocweave-lsp-<version>-<target>`形式のディレクトリへ置きます。同じ版を取得済みの場合は再取得せず、新しい版を取得したときは以前の版を削除します。

## 既知の制約

- 自動取得したLanguage Serverの版を拡張の設定から固定できません。常に最新の安定版を取得します。
- 自動取得ではattestationを検証しません。checksumは照合します。
- Marketplaceの索引更新には遅れがあり、公開直後は拡張画面へ現れないことがあります。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
