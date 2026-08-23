---
name: github-prose
description: GitHubのIssue、Pull Requestまたはそのコメントとレビューへ文章を投稿するときに使用します。禁止語検査を通してから投稿する手順を示します。
---

# GitHubへの投稿

このリポジトリでは、AIエージェントがGitHubへ送る題名と本文を、投稿の直前に禁止語検査へかけます。検査は `config/japanese-terminology.json` の用語一覧を正本とし、リポジトリのAsciiDoc文書と同じ規則を使います。

`gh` で題名や本文を直接送ることはできません。`PreToolUse` hookが該当するコマンドを拒否します。代わりに `tools/checked-gh-prose.mjs` を使用してください。検査に通った題名と本文だけが `gh` へ渡ります。

## 手順

1. 本文をファイルへ書き出します。改行や記号がshellで壊れないため、`--body` へ直接渡すより確実です。
2. ラッパー経由で投稿します。

   ```console
   node tools/checked-gh-prose.mjs issue create \
     --title '題名です' --body-file /tmp/body.md
   node tools/checked-gh-prose.mjs pr comment 711 --body-file /tmp/comment.md
   ```

3. 禁止語が見つかると `field:行:桁: メッセージ` の形式で報告し、`gh` は実行しません。表現を直してから同じコマンドを再実行します。

## 使用できる操作

`issue` の `create`、`edit`、`comment` と、`pr` の `create`、`edit`、`comment`、`review` に対応します。

題名と本文は `--title`、`--body` または `--body-file` で明示します。`create` では題名と本文の両方が必要です。短縮形の `-t`、`-b`、`-F`、標準入力を指す `--body-file -`、および `--editor`、`--web`、`--fill` などの対話的なoptionは、検査対象を確定できないため拒否します。

## 検査を伴わない操作

`gh issue view`、`gh pr checks`、`gh issue edit 1 --add-label bug`、`gh pr review 1 --approve` のように文章を送らないコマンドは、これまでどおり `gh` をそのまま実行します。

## 投稿前に文章だけを確認する

投稿せずに検査結果だけを見る場合は、検査部品を直接実行します。

```console
node tools/textlint/github-markdown-lint.mjs body /tmp/body.md
```

## 仕組み

- `tools/github-prose-hook.mjs` — `gh` による直接投稿を検出して拒否する判定です。CodexとClaude Codeで共有します。
- `tools/checked-gh-prose.mjs` — 引数を解釈し、検査に通った題名と本文から `gh` の引数を組み直します。
- `tools/textlint/github-markdown-lint.mjs` — Markdownを解析し、コードとURLを除いた本文へ用語規則を適用します。
