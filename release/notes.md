# AdocWeave lsp v0.46.2

## 主な変更

- **走査が終えられなかったことの通知を、文書の診断からエディターへのメッセージに変えました。** v0.46.1では、原因のない文書を含むすべてのファイルへ警告を表示していました。今後は``window/showMessage``のWarningとして一度だけ通知します。

## 対応環境

Linux、macOSおよびWindows向けのLanguage Serverを配布します。対応するOSとCPUは配布manifestで確認できます。

## 対応関係

エディター拡張との互換性は、製品バージョンではなく``lspApiVersion``で判断してください。

## v0.46.2への移行

- ``workspace-scan-incomplete``の診断を処理していた場合は、``window/showMessage``の受信へ切り替えてください。

## 更新とロールバック

新しいLanguage Serverをバージョン別directoryへ展開し、`--version --json`の結果を確認してから利用先を切り替えてください。受入確認が終わるまで以前のdirectoryを保持すると、問題がある場合に元へ戻せます。

## 既知の制約

- 初期ワークスペース走査が終わるまでは、開いた文書の解析にワークスペース内のほかの文書が反映されません。走査完了後に再解析します。
- ``workspace.scan.exclude``は初期走査だけに適用します。明示的に開いた文書、file watcherの通知およびinclude先を拒否する設定ではありません。

## 配布物の検証

対象Releaseの`sha256.sum`でarchiveを検査し、`gh attestation verify <asset> --repo KeishiS/adocweave`でattestationを検証してください。
