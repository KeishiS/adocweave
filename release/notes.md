# AdocWeave v0.46.0

## 主な変更

- **``include::``を既定で展開するようになりました。** これまでは``resources.include``を書くか``--include``を指定する必要があり、しかも``.adocweave.toml``を置くと展開が止まっていました。lint規則を一つ設定しただけでinclude解決が無効になる、という分かりにくい挙動を解消します。展開したくない場合は``--no-include``または``include = false``を指定してください。
- **Language Serverの初期走査に、既定の除外ディレクトリを設けました。** ``workspace.scan.exclude``を書いていない場合、``**/.git``、``**/.venv``、``**/node_modules``、``**/target``を走査から外します。設定を書かないまま走査上限に達して、解析が始まらない事象を防ぎます。
- **走査が上限に達しても、Language Serverが動作を続けるようになりました。** これまではワークスペース全体を破棄していたため、編集中のファイルの診断や整形まで止まっていました。上限に達した時点までに見つけた文書を解析対象とし、警告(``workspace-scan-incomplete``)で``workspace.scan.exclude``による除外を案内します。
- browser packageのTypeScript宣言を、Rustの型定義から生成するようにしました。型名が変わります。詳細は「公開契約と破壊的変更」を参照してください。
- CLIが入力の走査上限に達したときに、上限値と到達したパスを表示するようになりました。

## 対応環境

CLIとLanguage Serverのnative archiveは、次に挙げるLinux、macOSおよびWindowsのtargetへ配布します。macOSとWindowsのbinaryはOSのsystem libraryへ動的linkします。browser package、Zed拡張、VS Code拡張およびtextlint用Processorはtargetに依存しません。

## 公開契約と破壊的変更

**browser packageのTypeScript型名が変わります。** 型宣言をRustのwire型から生成するようにしたため、公開する名前が生成器の出力名になります。要求は``WasmRequest``、応答は``WasmResponse``です。``web-worker/index.d.mts``が従来名の別名を提供するため、packageの入口から取り込んでいる場合は変更不要です。``protocol.generated.d.mts``を直接参照している場合は``protocol.d.mts``へ変更してください。

**Worker messageの実行時検証が、外側の封筒だけになります。** これまでJavaScript側が要求の全項目を検査していましたが、同じ判定をRustのserdeが行うため、二重の実装をやめました。不正な要求はWASM側が同じ理由で拒否するため、拒否する入力の集合は変わりません。

**``resources.include``の既定値が``false``から``true``へ変わります。** 設定を書いていないプロジェクトでは、CLIとLanguage Serverがinclude先を読み込むようになります。読み込める範囲は従来どおり``resources.roots``、workspace境界およびファイルシステムの権限が決めるため、この変更で新しいパスは開きません。標準入力から読んだ文書は例外で、位置を持たないため``--base-dir``を指定した場合だけ展開します。

**CLIへ``--no-include``を追加しました。** ``--include``は、設定で無効にしている場合に一度だけ有効化するoptionとして残ります。両方を同時に指定した場合はusage errorです。

**初期走査が上限に達したときの診断が変わります。** 従来はerror(``workspace-resource-error``)でワークスペース全体を破棄していました。今後は警告(``workspace-scan-incomplete``)を公開し、そこまでに見つけた文書で動作を続けます。権限違反や不正な設定は従来どおりerrorで、結果を公開しません。

**``adocweave config show``の出力が変わります。** ``resources.include``が既定で``true``、``workspace.scan.exclude``が既定の4件を返します。

WASM protocol schema version（14）とWorker protocol version（2）は、どちらもv0.45.0から変更していません。AsciiDocの解析結果とHTML出力も変更していません。textlint Processorの公開API、TxtASTへの変換結果、自動修正を行わない保証、パッケージの構成および対応するtextlintの版も変更していません。GitHub Release以外のregistryへpackageを公開せず、VS Code拡張だけをOpen VSXへ公開する方針も変わりません。

consumerは記載されたpackage versionを厳密に一致させてください。異なるversionのCLI、LSP、browser、Zed、VS Codeまたはtextlint向け配布物を混在させないでください。

## v0.46.0への移行

- Browser向けWASMのJSON requestを直接構築している場合は、0.46.0のpackageとAPIへ更新し、requestの``packageVersion``も``0.46.0``にそろえてください。``schemaVersion``はrequestの項目ではありません。requestには追加しないでください。保存済みの結果やcacheは、packageが公開する``PROTOCOL_SCHEMA_VERSION = 14``を使って区別してください。
- browser packageのTypeScript宣言を直接参照している場合は、``protocol.generated.d.mts``を``protocol.d.mts``へ変更し、型名を``WasmRequest``と``WasmResponse``へそろえてください。packageの入口から取り込んでいる場合は変更不要です。
- include先を読ませたくないプロジェクトは、``.adocweave.toml``へ``[resources]``の``include = false``を明示してください。これまで``resources.include``を書かずに「展開しない」状態を利用していた場合、この版から展開します。
- 初期走査の除外を明示しているプロジェクトは変更不要です。除外を一つも設けたくない場合は``exclude = []``と空のリストを指定してください。
- CLI、LSP、browser、Zed、VS Codeおよびtextlint向け配布物のversionを0.46.0へそろえてください。バージョンの異なる配布物を混ぜて使えないため、更新する場合はすべてを入れ替えます。

## 更新とロールバック

native archiveはversion別directoryへ展開し、`--version --json`が`0.46.0`を返すことを確認してから選択先を切り替えてください。

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
- Language Serverの初期ワークスペース走査は、複数project scopeの合計で設定と本文の読込10,000回、取得内容50 MiB、directory entry 100,000件、候補変更10,000回、参加session 10,002件までです。project設定のsession単位の上限も別に適用します。directory entryの上限に達した場合は、そこまでに見つけた文書で動作を続け、警告で報告します。
- ``workspace.scan.exclude``はLanguage Serverの初期走査だけに適用します。CLI入力、明示的に開いた文書、file watcherの通知およびinclude先を拒否する設定ではありません。

## 配布物の検証

すべてのrelease assetをdownloadし、`sha256sum --check sha256.sum`を実行してください。その後、必要なassetを`gh attestation verify <asset> --repo KeishiS/adocweave`で検証してください。配布manifest（`adocweave-dist-manifest.json`）とSPDX SBOMには、公開したarchiveの名前、対象環境、byte数およびSHA-256を記録しています。
