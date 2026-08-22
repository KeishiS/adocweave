# AdocWeave v0.46.1

## 主な変更

- **プロジェクト設定の読み込み上限で、ワークスペース全体の解析が止まる不具合を修正しました。** ある``.adocweave.toml``の``resources.max-files``が、そのディレクトリ以下の文書数より小さい場合、Language Serverの初期走査が最初の拒否でワークスペース全体の読み込みを失敗させ、無関係なファイルまで解析できなくなっていました。今後は読めなかった文書を飛ばして走査を続け、上限を設けている設定ファイルを名指しした警告(``workspace-scan-incomplete``)を報告します。文書を開いたときの解析では、これらの上限を従来どおり適用します。

## 対応環境

CLIとLanguage Serverのnative archiveは、次に挙げるLinux、macOSおよびWindowsのtargetへ配布します。macOSとWindowsのbinaryはOSのsystem libraryへ動的linkします。browser package、Zed拡張、VS Code拡張およびtextlint用Processorはtargetに依存しません。

## 公開契約と破壊的変更

v0.46.0からの破壊的変更はありません。この版で変わるのは、初期走査が読み込み上限に達したときのLanguage Serverの状態です。従来はerror(``workspace-resource-error``)でワークスペース全体を破棄していましたが、読めた文書を保持し、警告(``workspace-scan-incomplete``)を報告します。severityとcodeは、v0.46.0で走査の項目数上限に導入したものと同じです。

WASM protocol schema version（14）とWorker protocol version（2）は、どちらもv0.46.0から変更していません。AsciiDocの解析結果とHTML出力も変更していません。textlint Processorの公開API、TxtASTへの変換結果、自動修正を行わない保証、パッケージの構成および対応するtextlintの版も変更していません。GitHub Release以外のregistryへpackageを公開せず、VS Code拡張だけをOpen VSXへ公開する方針も変わりません。

consumerは記載されたpackage versionを厳密に一致させてください。異なるversionのCLI、LSP、browser、Zed、VS Codeまたはtextlint向け配布物を混在させないでください。

## v0.46.1への移行

- Browser向けWASMのJSON requestを直接構築している場合は、0.46.1のpackageとAPIへ更新し、requestの``packageVersion``も``0.46.1``にそろえてください。``schemaVersion``はrequestの項目ではありません。requestには追加しないでください。保存済みの結果やcacheは、packageが公開する``PROTOCOL_SCHEMA_VERSION = 14``を使って区別してください。
- CLI、LSP、browser、Zed、VS Codeおよびtextlint向け配布物のversionを0.46.1へそろえてください。バージョンの異なる配布物を混ぜて使えないため、更新する場合はすべてを入れ替えます。
- v0.46.0でこの不具合に該当していた場合、更新後は該当プロジェクトの文書だけが登録されず、警告で報告されます。すべての文書を登録するには、警告が名指しする``.adocweave.toml``の``resources.max-files``または byte 数の上限を引き上げてください。

## 更新とロールバック

native archiveはversion別directoryへ展開し、`--version --json`が`0.46.1`を返すことを確認してから選択先を切り替えてください。

VS Codeでは検証済みVSIXを手動導入し、拡張とLanguage Serverのversion一致を確認してください。受入確認が成功するまで以前のVSIXとnative directoryを保持します。

Zedでは新versionのmanaged Language Server取得とeditor機能を確認するまで旧versionのZed directoryを保持します。rollback時は旧directoryをdev extensionとして選び直し、Zedを再起動してください。

textlint用Processorは新しいReleaseのtarball URLへ変更してlockfileを更新します。rollback時は以前の検証済みURLへ戻し、lockfileから依存を再導入してください。

rollback時は以前のversion別directoryまたはVSIXへ戻します。詳細は`docs/user-guide/release-installation.adoc`を参照してください。

## 既知の制約

- native binaryは配布計画に定義したLinux、macOSおよびWindows環境へ提供します。macOS binaryへDeveloper ID署名とnotarizationを行わず、Windows binaryへAuthenticode署名を行いません。OSの警告が表示された場合はchecksumとattestationを確認してください。
- Zed拡張はdevelopment extensionとして手動導入します。Zed Extension Galleryへは公開しません。
- VS Code拡張はOpen VSXだけへ公開します。Visual Studio Marketplaceへは公開しないため、Visual Studio Code本体では検証済みVSIXを手動で導入します。
- Open VSXへ公開した版は取り消しません。問題が見つかった場合は、新しいversionを公開して置き換えます。
- ZedがLanguage Serverの導入中に異常終了すると、安全のため導入ロックを自動削除しません。すべてのZedプロセスを終了してから、エラーに表示されたロックのpathを削除して再試行してください。
- 公式Playgroundはこのreleaseに含みません。`adocweave preview`は利用者の端末で実行するローカル機能です。
- packageはcrates.io、npmまたはOS package registryへ公開しません。Nix packageはこのrepositoryのflakeから直接buildします。
- textlint用Processorはincludeを展開せず、入力した一つの物理ファイルだけを検査します。
- AdocWeaveはBibTeXの保存・解析やCSL相当の書誌の組版を行いません。citation keyの解決と引用表示の組み立ては利用側アプリの責務です。
- 解決結果を渡さない引用の表示は`unresolved_references`の設定に従い、`hidden`では出力しません。ただし文書内の`[bibliography]`項目を指すkeyは、設定にかかわらずその項目へのlinkとして出力します。
- 引用の解決結果は文書全体の並べ替えを行いません。番号付きの引用styleで通し番号を振る場合は、利用側アプリが出現順を見て文字列を決めてください。出現順は公開projectionの`citations`から取得できます。
- 単一ファイルのworkspaceでは、同じディレクトリの別のAsciiDocファイルとinclude先を自動では読み込みません。複数ファイルの解析にはディレクトリのworkspace folderが必要です。
- Language Serverはworkspaceの走査を初期化の応答後に、要求へ応答するthreadの外で行います。走査中もほかの要求へ応答しますが、走査の完了前は、開いた文書の解析にworkspace内のほかの文書が反映されません。走査の完了後に再解析します。
- Linuxでfilesystemのhandle相対競合耐性を利用するには、``/proc/self/fd``を読み取れる実行環境が必要です。利用できない場合や、開いたfileのpathとidentityを確認できない場合は、安全性の低いpath検査へ切り替えず、``local-target-unverifiable``としてworkspaceの読込を拒否します。macOSとWindowsは、同時変更のない静的なfilesystem snapshotだけを前提とします。
- 一つのfilesystem policyが保持できるrootは128件までです。読込対象を増やす場合は、設定のrootを必要な上位directoryへまとめてください。
- Language Serverの初期ワークスペース走査は、複数project scopeの合計で設定と本文の読込10,000回、取得内容50 MiB、directory entry 100,000件、候補変更10,000回、参加session 10,002件までです。project設定のsession単位の上限も別に適用します。directory entryの上限、およびproject設定の``resources``による読み込み上限に達した場合は、そこまでに見つけた文書で動作を続け、警告で報告します。
- ``workspace.scan.exclude``はLanguage Serverの初期走査だけに適用します。CLI入力、明示的に開いた文書、file watcherの通知およびinclude先を拒否する設定ではありません。

## 配布物の検証

すべてのrelease assetをdownloadし、`sha256sum --check sha256.sum`を実行してください。その後、必要なassetを`gh attestation verify <asset> --repo KeishiS/adocweave`で検証してください。配布manifest（`adocweave-dist-manifest.json`）とSPDX SBOMには、公開したarchiveの名前、対象環境、byte数およびSHA-256を記録しています。
