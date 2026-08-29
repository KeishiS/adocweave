# AdocWeave vscode v0.47.1

## 主な変更

- **必要なVisual Studio Codeの版を1.91.0以降へ下げました。** 0.47.0は1.125.0以降を要求し、それより前の版では「not compatible with the current version of visual studio code」として導入できませんでした。この下限には根拠がなく、実際に必要な版より34版ぶん高い値でした。

拡張が使うVisual Studio CodeのAPIのうち最も新しいものは1.74で入りました。依存する`vscode-languageclient`が要求するのは1.91.0以降です。この2つから、実際の下限は1.91.0です。

機能の変更はありません。0.47.0を導入できている場合、この版で動作は変わりません。

## 対応環境

Visual Studio Code 1.91.0以降が必要です。拡張自体はplatformを選びませんが、別途導入するLanguage Serverは配布対象のtargetに従います。対応はlinuxのx86-64とARM64、macOSのARM64、Windowsのx86-64です。

宣言した下限は、その版の実Visual Studio Codeで検証しています。拡張の起動、VSIXの導入、更新、rollbackおよび削除を確認しました。

## 対応関係

拡張とLanguage Serverは独立した製品で、版を揃える必要はありません。利用できる機能は、接続時に交換する標準LSP capabilityから決まります。

## v0.47.1への移行

- 利用者の作業は不要です。0.47.0を導入できていた環境では、拡張画面から通常どおり更新できます。
- 0.47.0を導入できなかった環境では、この版から導入できます。Visual Studio Codeを更新する必要はありません。

## 更新とロールバック

Visual Studio MarketplaceまたはOpen VSX Registryから導入した場合は、editorの拡張画面から更新および以前の版への切り替えができます。VSIXを手動で導入した場合は、新しいVSIXを`code --install-extension <file> --force`で導入し、Windowを再読込します。

## 既知の制約

- 拡張はLanguage Serverを同梱せず、取得もしません。別途導入が必要です。
- 未信頼ワークスペースでは拡張を起動しません。設定はmachine scopeに限定します。
- Marketplaceの索引更新には遅れがあり、公開直後は拡張画面へ現れないことがあります。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
