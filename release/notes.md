# AdocWeave vscode v0.47.0

## 主な変更

- **Visual Studio Marketplaceから導入できます。** これまでVisual Studio Code本体では、GitHub ReleaseのVSIXを取得して手動で導入する必要がありました。拡張画面から`adocweave.adocweave-vscode`を検索して導入、更新および削除できます。公式の発行者はpublisher`adocweave`です。
- **拡張はLanguage Serverを取得しなくなりました。** 以前の版は`adocweave-lsp`をeditorの保存領域へdownloadして管理していましたが、この処理を削除しました。利用者が導入した実行ファイルを、設定の絶対pathまたは`PATH`から探します。見つからない場合は導入案内を表示し、起動しません。
- **拡張とLanguage Serverの版を一致させる必要がなくなりました。** 利用できる機能は、接続時に交換する標準LSP capabilityから決まります。`lspApiVersion`の一致を要求しません。

Open VSX Registryへも同じVSIXを公開します。どちらのregistryから導入しても、GitHub Releaseへ添付したfileと同一です。

## 対応環境

Visual Studio Code 1.125.0以降が必要です。拡張自体はplatformを選びませんが、別途導入するLanguage Serverは配布対象のtargetに従います。対応はlinuxのx86-64とARM64、macOSのARM64、Windowsのx86-64です。

## 対応関係

拡張とLanguage Serverは独立した製品で、版を揃える必要はありません。診断、補完および定義への移動などの利用可否は、Language Serverが返す標準LSP capabilityから判断します。標準capabilityがない機能は、editorのLSP clientが有効にしません。

## v0.47.0への移行

- 以前の版が自動取得したLanguage Serverを使っていた場合は、`adocweave-lsp`を自分で導入し、`PATH`へ通すか`adocweave.server.path`へ絶対pathを設定してください。導入手順は利用者向けの配布物導入文書を参照してください。
- 更新前に、旧版で`adocweave.server.download`を`false`にしてから`AdocWeave: Remove Managed Language Server`を実行すると、使われなくなるキャッシュを確実に削除できます。更新後に気付いた場合の削除手順も同じ文書にあります。
- Marketplaceから導入し直す場合は、手動で入れたVSIXを先に削除してください。同じ識別子のため、二重には入りません。

## 更新とロールバック

Marketplaceまたは Open VSXから導入した場合は、editorの拡張画面から更新および以前の版への切り替えができます。VSIXを手動で導入した場合は、新しいVSIXを`code --install-extension <file> --force`で導入し、Windowを再読込します。rollbackでは以前のReleaseの検証済みVSIXを同じ方法で導入します。

## 既知の制約

- 拡張はLanguage Serverを同梱せず、取得もしません。別途導入が必要です。
- 未信頼ワークスペースでは拡張を起動しません。設定はmachine scopeに限定します。
- Marketplaceの索引更新には遅れがあり、公開直後は拡張画面へ現れないことがあります。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
