#!/usr/bin/env bash
set -eu

site_dir="${1:-_site}"

case "$site_dir" in
  "" | "/" | ".")
    echo "Refusing to replace unsafe site directory: $site_dir" >&2
    exit 2
    ;;
esac

rm -rf "$site_dir"
mkdir -p "$site_dir/releases" "$site_dir/assets" "$site_dir/images" "$site_dir/examples"

cp docs/*.html docs/styles.css docs/app.js "$site_dir/"
cp docs/images/quillspeak-*.png "$site_dir/images/"
cp docs/examples/* "$site_dir/examples/"
cp assets/icons/hicolor/scalable/apps/quillspeak.svg "$site_dir/assets/quillspeak.svg"
touch "$site_dir/.nojekyll"

if [ -n "${GITHUB_REPOSITORY:-}" ] && [ -n "${GH_TOKEN:-}" ] && command -v gh >/dev/null 2>&1; then
  if gh api "repos/${GITHUB_REPOSITORY}/releases/latest" > "$site_dir/releases/latest.json"; then
    exit 0
  fi
fi

printf '{"message":"No release metadata available yet","assets":[]}\n' > "$site_dir/releases/latest.json"
