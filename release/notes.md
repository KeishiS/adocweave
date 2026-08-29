# AdocWeave vscode v0.47.2

## 主な変更

- **必要なVisual Studio Codeの版を1.107.0以降へ下げました。** 0.47.0は1.125.0以降を要求し、それより前の版では「not compatible with the current version of visual studio code」として導入できませんでした。1.107.0以降であれば導入できます。

機能の変更はありません。0.47.0を導入できている場合、この版で動作は変わりません。

下限を1.107.0とした理由を説明します。依存する`vscode-languageclient`が宣言する下限は1.91.0ですが、その版では拡張がCode Actionを返しません。宣言上の下限と、機能が実際に揃う版は別でした。実際のVisual Studio Codeで順に確かめ、Code Actionを含むすべての機能が揃う最小の版が1.107.0であることを確認しています。1.106.0では揃いません。

この版から、検査に使うVisual Studio Codeの版を`engines.vscode`から導きます。宣言した対応範囲の下限そのもので、拡張の起動、診断、補完、定義への移動、Code Action、rename、semantic tokens、およびVSIXの導入、更新、rollback、削除を検証します。

## 対応環境

Visual Studio Code 1.107.0以降が必要です。拡張自体はplatformを選びませんが、別途導入するLanguage Serverは配布対象のtargetに従います。対応はlinuxのx86-64とARM64、macOSのARM64、Windowsのx86-64です。

## 対応関係

拡張とLanguage Serverは独立した製品で、版を揃える必要はありません。利用できる機能は、接続時に交換する標準LSP capabilityから決まります。

## v0.47.2への移行

- 利用者の作業は不要です。導入済みの環境では、拡張画面から通常どおり更新できます。
- 0.47.0を導入できなかった1.107.0以降の環境では、この版から導入できます。Visual Studio Codeを更新する必要はありません。
- 1.106.0以前をお使いの場合は、Visual Studio Codeの更新が必要です。

## 更新とロールバック

Visual Studio MarketplaceまたはOpen VSX Registryから導入した場合は、editorの拡張画面から更新および以前の版への切り替えができます。VSIXを手動で導入した場合は、新しいVSIXを`code --install-extension <file> --force`で導入し、Windowを再読込します。

## 既知の制約

- 拡張はLanguage Serverを同梱せず、取得もしません。別途導入が必要です。
- 未信頼ワークスペースでは拡張を起動しません。設定はmachine scopeに限定します。
- Marketplaceの索引更新には遅れがあり、公開直後は拡張画面へ現れないことがあります。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
