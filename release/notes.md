# AdocWeave textlint v0.47.0

## 主な変更

- **`engines`と`peerDependencies`を範囲指定へ緩めました。** 従来はNode.js 24.19.0とtextlint 15.8.0へ完全一致で固定しており、動作する組合せでも導入時に警告や失敗が起きていました。今後はNode.js 24.19.0以上と、textlint 15.8系を受け入れます。CIで確認する組合せは変えていません。
- **READMEをMarkdownへ変更しました。** package registryのpackageページで描画できる形式にします。記載内容は変えていません。

Processorの公開interface、TxtASTへの変換結果および診断の位置は変えていません。

## 対応環境

Node.js 24.19.0以上と、textlint 15.8系で動作します。WebAssemblyはパッケージへ同梱するため、Rust、Cargoまたは実行時の追加ダウンロードを必要としません。

## 対応関係

textlintのProcessor Pluginとして動作します。AdocWeaveのほかの製品を同じ環境へ導入する必要はありません。

## v0.47.0への移行

- 利用者の作業は不要です。依存の指定方法と設定は変わりません。
- Node.jsまたはtextlintのversionが合わず導入できなかった場合は、範囲指定へ緩めたことで導入できるようになります。

## 更新とロールバック

依存として記録したversionを新しい版へ変更し、`npm install`を実行してから`package-lock.json`の差分と検査結果を確認してください。以前の版へ戻す場合は、同じ手順で戻したいversionを指定します。受入確認が終わるまで以前の`package-lock.json`を保持すると、問題がある場合に元へ戻せます。

## 既知の制約

- includeを展開せず、入力された一つの物理ファイルだけを解析します。
- 自動修正に対応しません。規則が修正情報を返した場合も削除するため、`textlint --fix`でAsciiDoc文書を書き換えません。
- 一つの入力は10 MiB、TxtAST planは50 MiB、planのnodeは1,000,000件、`sourceId`は4 KiBを上限とします。同梱WebAssemblyのlinear memoryは256 MiBを上限とします。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
