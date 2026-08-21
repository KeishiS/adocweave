# AdocWeave for Visual Studio Code

AsciiDocの基本構文色付けと、`adocweave-lsp`による診断、補完、移動、整形およびSemantic Tokensを提供します。

## Language Serverの選択

拡張は、明示設定、`PATH`、検証済みmanaged binaryの順でLanguage Serverを選択します。managed binaryは、拡張と同じversionのGitHub Releaseからdownloadし、checksumを検証してから使用します。workspaceの設定値は、Visual Studio Codeがそのworkspaceを信頼している場合だけ使用します。

## 導入

検証済みVSIXをGitHub Releases（<https://github.com/KeishiS/adocweave/releases>）から取得し、手動で導入します。拡張とLanguage Serverのversionは一致させてください。導入、更新、rollbackおよび検証の手順は、リポジトリの`docs/user-guide/release-installation.adoc`を参照してください。

Visual Studio MarketplaceおよびOpen VSXへは公開していません。

## ライセンス

MIT OR Apache-2.0
