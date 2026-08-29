#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "使用方法: tools/normalize-darwin-archives.sh ARTIFACT_DIRECTORY DIST_PLAN TARGET" >&2
  exit 2
fi

artifact_directory="$(cd "$1" && pwd)"
plan_file="$2"
target="$3"
case "$target" in
  *-apple-darwin) ;;
  *)
    echo "Darwin以外のtargetは正規化できません: $target" >&2
    exit 2
    ;;
esac

if [ ! -f "$plan_file" ]; then
  echo "cargo-distの配布計画がありません: $plan_file" >&2
  exit 2
fi

if ! selected="$(
  jq -er --arg target "$target" \
    '[.releases[]? | select(.app_name == "adocweave")] as $releases |
     [.artifacts[]? |
       select(.kind == "executable-zip" and .target_triples == [$target])] as $artifacts |
     if ($releases | length) != 1 or ($artifacts | length) != 1 then
       error("Darwin archive selection must resolve to one release and one target")
     else
       $artifacts[0] as $artifact |
       [$artifact.assets[]? | select(.kind == "executable")] as $executables |
       if $artifact.name != ("adocweave-" + $target + ".zip") or
          ($executables | length) != 1 or
          $executables[0].name != "adocweave" or
          $executables[0].path != "adocweave" then
         error("Darwin archive does not match the native distribution contract")
       else
         [$artifact.name, $executables[0].path] | @tsv
       end
     end' \
    "$plan_file"
)"; then
  echo "cargo-distの配布計画からDarwin archiveを一つに特定できません: $target" >&2
  exit 2
fi
IFS=$'\t' read -r archive_name executable <<< "$selected"

scratch="$(mktemp -d "${TMPDIR:-/tmp}/adocweave-darwin-archives.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

archive="$artifact_directory/$archive_name"
if [ ! -f "$archive" ]; then
  echo "Darwin archiveがありません: $archive" >&2
  exit 1
fi

destination="$scratch/archive"
mkdir "$destination"
unzip -q "$archive" -d "$destination"
binary="$destination/$executable"

while IFS= read -r dependency; do
  case "$dependency" in
    /nix/store/*-libiconv-*/lib/libiconv.*.dylib)
      install_name_tool -change "$dependency" /usr/lib/libiconv.2.dylib "$binary"
      ;;
  esac
done < <(otool -L "$binary" | tail -n +2 | awk '{print $1}')

if otool -L "$binary" | tail -n +2 | awk '{print $1}' | grep -q '^/nix/store/'; then
  echo "Darwin実行ファイルにNix storeの動的依存が残っています: $executable" >&2
  exit 1
fi

normalized="$scratch/normalized-$archive_name"
(
  cd "$destination"
  zip -q -X "$normalized" ./*
)
mv "$normalized" "$archive"
