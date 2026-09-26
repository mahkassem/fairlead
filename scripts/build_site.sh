#!/usr/bin/env bash
# Builds the published site into _site/: the landing page at the root, the book
# under docs/, the brand files, and a redirect for every chapter's old root URL.
set -euo pipefail
cd "$(dirname "$0")/.."
mdbook="${MDBOOK:-mdbook}"
rm -rf _site
"$mdbook" build docs -d "$PWD/_site/docs"
cp -R site/. _site/
mkdir -p _site/brand
cp -R assets/brand/. _site/brand/
rm -f _site/brand/README.md
cp _site/docs/404.html _site/404.html
for page in _site/docs/*.html; do
  name=$(basename "$page")
  case "$name" in index.html|404.html|toc.html) continue ;; esac
  [ -e "_site/$name" ] && continue
  printf '<!doctype html><meta charset="utf-8"><title>Moved</title><link rel="canonical" href="docs/%s"><meta http-equiv="refresh" content="0; url=docs/%s"><p><a href="docs/%s">This page moved to docs/%s</a>.</p>\n' \
    "$name" "$name" "$name" "$name" > "_site/$name"
done
# JSON files (the plan schema, the benchmark numbers) stay at their old root URLs
# too, since a redirect page doesn't help a program fetching them.
for file in _site/docs/*.json; do
  [ -e "$file" ] && cp "$file" "_site/$(basename "$file")"
done
echo "site built in _site/"
