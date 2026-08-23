#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "使用方法: tools/normalize-darwin-archives.sh ARTIFACT_DIRECTORY PRODUCT TARGET" >&2
  exit 2
fi

artifact_directory="$(cd "$1" && pwd)"
product="$2"
target="$3"
case "$target" in
  *-apple-darwin) ;;
  *)
    echo "Darwin以外のtargetは正規化できません: $target" >&2
    exit 2
    ;;
esac

if ! jq -e --arg target "$target" \
  '.targets | any(.triple == $target and .os == "darwin")' \
  release/distribution-plan.json >/dev/null; then
  echo "配布計画にDarwin targetがありません: $target" >&2
  exit 2
fi

if ! jq -e --arg product "$product" \
  '.products | any(.product == $product and .build == "cargo-dist" and .executable != null)' \
  release/distribution-plan.json >/dev/null; then
  echo "Darwin実行fileを持つ製品ではありません: $product" >&2
  exit 2
fi

while IFS= read -r other_archive_name; do
  if [ -e "$artifact_directory/$other_archive_name" ]; then
    echo "別製品のDarwin archiveが混在しています: $artifact_directory/$other_archive_name" >&2
    exit 1
  fi
done < <(
  jq -r --arg product "$product" --arg target "$target" \
    '. as $plan |
     $plan.products[] |
       select(.product != $product and .build == "cargo-dist" and .executable != null) as $route |
     $plan.targets[] | select(.triple == $target) as $platform |
     $route.assetName | gsub("\\{target\\}"; $platform.triple)' \
    release/distribution-plan.json
)

selected="$(
  jq -er --arg product "$product" --arg target "$target" \
    '. as $plan |
     [$plan.products[] |
       select(.product == $product and .build == "cargo-dist" and .executable != null)] as $routes |
     [$plan.targets[] | select(.triple == $target and .os == "darwin")] as $platforms |
     if ($routes | length) != 1 or ($platforms | length) != 1 then
       error("Darwin archive selection must resolve to one product and one target")
     else
       $routes[0] as $route |
       $platforms[0] as $platform |
       [($route.assetName | gsub("\\{target\\}"; $platform.triple)),
        ($route.executable | gsub("\\{executableSuffix\\}"; $platform.executableSuffix))] | @tsv
     end' \
    release/distribution-plan.json
)"
IFS=$'\t' read -r archive_name executable <<< "$selected"

scratch="$(mktemp -d "${TMPDIR:-/tmp}/adocweave-darwin-archives.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

archive="$artifact_directory/$archive_name"
if [ ! -f "$archive" ]; then
  echo "Darwin archiveがありません: $archive" >&2
  exit 1
fi

destination="$scratch/$executable"
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
  echo "Darwin実行fileにNix storeの動的依存が残っています: $executable" >&2
  exit 1
fi

normalized="$scratch/normalized-$archive_name"
(
  cd "$destination"
  zip -q -X "$normalized" ./*
)
mv "$normalized" "$archive"
