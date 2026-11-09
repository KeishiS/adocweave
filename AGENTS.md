# リポジトリの作業指針

## プロジェクトの概要

本リポジトリの AdocWeave はAsciiDoc文書の整形、静的解析、HTML変換などの周辺ツールを提供します。
基本的にはAsciiDocの標準仕様に準拠しながら、Asciidoctorとの互換性も確認しながら実装しています。
現時点では以下の機能を提供します。

* AsciiDocからHTMLへの変換
* 数式や引用を含むreveal.jsスライドの作成
* ターミナルでのレンダリング
* AsciiDocファイルの静的解析と整形
* textlint連携での静的解析
* VS CodeとZedのためのlanguage Server
* ブラウザとNode.jsで利用可能なWebAssemblyライブラリ

## 作業上の制約

* 見出しとブロックへ明示的なIDを付ける場合は `[#id]` 形式を使用し、 `[[id]]` 形式は本文中のアンカーに限定します。
* CLIが端末へ表示するhelp, info, error および診断は英語のメッセージで統一します。
* コードから読み取れる情報はドキュメントに記載せず、中核となるコンセプトや始めて使うユーザのためのわかりやすい動線を心がけます。
* 実装経緯や完了報告、一時的な計画はGitHub Issuesに記載し、ドキュメントには記載しません。
* developer向けの実装文書は処理の流れの理解促進を目的とするため、必要以上に内部構造の説明はしません。
* 作業は以下の流れに沿って進めます。
  1. 作業の目的を1つに絞ります。複数ある場合は、分割可能な単位に分割してGitHub Issuesを作成します。
  2. 実装上の未確定情報を洗い出し、ユーザに質問します。その際に以下の点を確認、考慮します。
    * 破壊的変更の可否
    * 後方互換性の有無
    * 保守性と可読性の考慮
    * 複雑性を抑制した、一貫性のある実装
    * セマンティックバージョニングに基づいたバージョン更新の扱い
  3. 設計を計画した段階でGitHub Issueを作成します。
  4. その設計を実現するための最小限の変更のための作業計画を考えます。
  5. ブランチを分けて作業を進めます。
  6. 分割可能な作業単位でGit Commitを行います。
  7. 計画に修正が生じた場合は、生じた問題と修正方針を該当Issueにコメントしてください。
  8. 完了次第、プルリクエストを作成してください。
  9. マージはユーザが行います。

## GitHubを使う際の制約

* GitHub Issueを作成する際は以下から適切なラベルを1つ以上設定してください。
  * `roadmap`: Tracking issue for a planned milestone or major feature
  * `bug`: Something isn't working
  * `dependencies`: Pull requests that update a dependency file
  * `documentation`: Improvements or additions to documentation
  * `enhancement`: New feature or request
  * `question`: Further information is requested 
* 複数の粒度の作業が必要な場合は `roadmap` ラベルを付与したGitHub Issueを作成してください。
* リリースタグは、リリース用Pull Requestが `main` へマージされて、対象コミットでリリースゲートが成功した後に作成してください。
* バージョン固有の変更はRelease Notesにまとめます。
* 長期的な設計判断と背景、結論は `docs/developer-guide/adr` に記載します。

## Git操作

* 環境変数の`GIT_AUTHOR_*`と`GIT_COMMITTER_*`を変更または削除しない。
* ブランチ名は `agent/<目的>` とし、直接 `main` へpushしない。 

## 検証

* commitの前に `makers verify` を実行し、成功することを確かめます。

## エージェントの設定

* 設計、実装、検証を独立したサブエージェントに担わせてください。
* 設計と検証においては `claude -p` でのOpus 5.5 または `codex exec` でのGPT-6 Astra の外部エージェントを使い、別の視点も確保します。
* 親エージェントは各作業の依存関係、差分および作業結果を確認し、統合に責任を持ちます。
