# AdocWeave vX.Y.Z

## 主な変更

- （利用者から見える変更を、変更の理由と影響が分かる文で箇条書きにします。1件目を記載してから公開）

## 対応環境

CLIとLanguage Serverのnative archiveは、次に挙げるLinux、macOSおよびWindowsのtargetへ配布します。macOSとWindowsのbinaryはOSのsystem libraryへ動的linkします。browser package、Zed拡張、VS Code拡張およびtextlint用Processorはtargetに依存しません。

（target一覧とRust／Node.jsの版は生成器が追記するため書きません。対応環境に変更がなければこの段落のまま公開できます。変更があればこの行を消し、内容を記載してから公開）

## 公開契約と破壊的変更

（WASM protocol schema versionとWorker protocol versionが前の版から変わったかを書きます。変えた場合は「WASM protocol schema versionをNからMへ更新しました。理由。」の形で書くと、到達値を生成器が正本と照合します。記載してから公開）

textlint Processorの公開API、TxtASTへの変換結果および自動修正を行わない保証は変更していません。GitHub Release以外のregistryへpackageまたは拡張を公開しません。

consumerは記載されたpackage versionを厳密に一致させてください。異なるversionのCLI、LSP、browser、Zed、VS Codeまたはtextlint向け配布物を混在させないでください。

## vX.Y.Zへの移行

- Browser向けWASMのJSON requestを直接構築している場合は、X.Y.ZのpackageとAPIへ更新し、requestの``packageVersion``も``X.Y.Z``にそろえてください。``schemaVersion``はrequestの項目ではありません。requestには追加しないでください。保存済みの結果やcacheは、packageが公開する``PROTOCOL_SCHEMA_VERSION``を使って区別してください。
- CLI、LSP、browser、Zed、VS Codeおよびtextlint向け配布物のversionをX.Y.Zへそろえてください。バージョンの異なる配布物を混ぜて使えないため、更新する場合はすべてを入れ替えます。
- （Rust APIの破壊的変更に伴う移行は``release/breaking-rust-api.json``から生成器が追記するため書きません。それ以外に利用者の作業が必要な変更があれば箇条書きで記載し、なければこの行を消してから公開）

## 更新とロールバック

native archiveはversion別directoryへ展開し、`--version --json`が`X.Y.Z`を返すことを確認してから選択先を切り替えてください。

VS Codeでは検証済みVSIXを手動導入し、拡張とLanguage Serverのversion一致を確認してください。受入確認が成功するまで以前のVSIXとnative directoryを保持します。

Zedでは新versionのmanaged Language Server取得とeditor機能を確認するまで旧versionのZed directoryを保持します。rollback時は旧directoryをdev extensionとして選び直し、Zedを再起動してください。

textlint用Processorは新しいReleaseのtarball URLへ変更してlockfileを更新します。rollback時は以前の検証済みURLへ戻し、lockfileから依存を再導入してください。

rollback時は以前のversion別directoryまたはVSIXへ戻します。詳細は`docs/user-guide/release-installation.adoc`を参照してください。

## 既知の制約

（前の版の``release/notes.md``の一覧を引き継ぎ、解消した制約を消し、新しい制約を足します。textlint用Processorの対応範囲は生成器が追記するため書きません。記載してから公開）

## 配布物の検証

すべてのrelease assetをdownloadし、`sha256sum --check sha256.sum`を実行してください。その後、必要なassetを`gh attestation verify <asset> --repo KeishiS/adocweave`で検証してください。配布manifest（`adocweave-dist-manifest.json`）とSPDX SBOMには、公開したarchiveの名前、対象環境、byte数およびSHA-256を記録しています。
