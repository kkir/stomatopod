#!/usr/bin/env bash
# Finish a stomatopod-www SSG output for GitHub Pages.
# Usage: scripts/prepare-www-artifact.sh [public-dir]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PUBLIC="${1:-$ROOT/target/dx/stomatopod-www/release/web/public}"

if [ ! -d "$PUBLIC" ]; then
  echo "error: SSG public dir missing: $PUBLIC" >&2
  exit 1
fi

cp "$ROOT/crates/www/public/robots.txt" "$PUBLIC/robots.txt"
cp "$ROOT/crates/www/public/sitemap.xml" "$PUBLIC/sitemap.xml"

# Stable share-card image (not a Dioxus-hashed asset).
mkdir -p "$PUBLIC/assets"
cp "$ROOT/crates/www/public/assets/og.png" "$PUBLIC/assets/og.png"

# Real 404 document. Do not copy index.html (that is the current live bug:
# unknown paths return homepage HTML with a 404 status).
if [ -f "$PUBLIC/404/index.html" ]; then
  cp "$PUBLIC/404/index.html" "$PUBLIC/404.html"
elif [ ! -f "$PUBLIC/404.html" ]; then
  echo "error: SSG did not emit 404/index.html or 404.html" >&2
  exit 1
fi

if cmp -s "$PUBLIC/index.html" "$PUBLIC/404.html"; then
  echo "error: 404.html must not be a copy of index.html" >&2
  exit 1
fi

# GitHub Pages URL resolution (verified live 2026-09-21 on stoma.top,
# jekyllrb.com, and simonw.github.io/playing-with-github-pages):
#
#   1. exact file
#   2. same path + ".html"          (clean URL; ".html" beats a directory)
#   3. directory index / slash redirect
#   4. root 404.html with HTTP 404
#
# So while root 404.html exists (required custom error document):
#   /missing, /404/     -> HTTP 404  (no file, no 404/index.html)
#   /404.html, /404     -> HTTP 200  (the error file itself, or clean-URL map)
#
# True HTTP 404 for GET /404 or GET /404.html is not available on Pages
# without deleting 404.html, which would drop the branded unknown-path
# body. A 404/ directory or extensionless "404" file would only add more
# 200s (or a 301 then 404), not remove the clean-URL 200.
# Mitigation: one error document, no 404/ directory, noindex, no sitemap
# URL. See https://github.com/kkir/stomatopod/issues/60
rm -rf "$PUBLIC/404"
if [ -e "$PUBLIC/404" ]; then
  echo "error: 404 must not remain as a file or directory" >&2
  exit 1
fi

# Persist custom domain across Actions deploys (apex canonical).
echo stoma.top > "$PUBLIC/CNAME"

page_html() {
  local name="$1"
  if [ "$name" = "index" ]; then
    echo "$PUBLIC/index.html"
  elif [ -f "$PUBLIC/$name/index.html" ]; then
    echo "$PUBLIC/$name/index.html"
  elif [ -f "$PUBLIC/$name.html" ]; then
    echo "$PUBLIC/$name.html"
  else
    echo "error: missing pre-rendered page $name" >&2
    exit 1
  fi
}

home="$(page_html index)"
features="$(page_html features)"
compare="$(page_html compare)"
get_started="$(page_html get-started)"

for f in "$home" "$features" "$compare" "$get_started"; do
  test -f "$f"
  grep -q '<link rel="canonical"' "$f"
  grep -q 'property="og:title"' "$f" || grep -q "property='og:title'" "$f"
  grep -q 'https://stoma.top/assets/og.png' "$f"
  grep -q 'property="og:image"' "$f" || grep -q "property='og:image'" "$f"
  grep -q 'summary_large_image' "$f"
  grep -q 'twitter:card' "$f"
  grep -q 'twitter:image' "$f"
  # Money pages stay indexable; noindex is only on the 404 template.
  ! grep -q 'noindex' "$f"
done

# Unique titles (Dioxus Title lands in <title>).
home_title="$(grep -o '<title>[^<]*</title>' "$home" | head -1)"
features_title="$(grep -o '<title>[^<]*</title>' "$features" | head -1)"
compare_title="$(grep -o '<title>[^<]*</title>' "$compare" | head -1)"
get_started_title="$(grep -o '<title>[^<]*</title>' "$get_started" | head -1)"
test -n "$home_title"
test "$home_title" != "$features_title"
test "$home_title" != "$compare_title"
test "$home_title" != "$get_started_title"
test "$features_title" != "$compare_title"
test "$features_title" != "$get_started_title"
test "$compare_title" != "$get_started_title"
echo "$compare_title" | grep -qi "open source"
echo "$compare_title" | grep -q "Plausible/Umami"

# Apex canonicals, slash-canonical paths.
grep -q 'https://stoma.top/' "$home"
grep -q 'https://stoma.top/features/' "$features"
grep -q 'https://stoma.top/compare/' "$compare"
grep -q 'https://stoma.top/get-started/' "$get_started"

# Home JSON-LD.
grep -q 'application/ld+json' "$home"
grep -q 'SoftwareApplication' "$home"

# Features: one H1 (demo chrome must not leak Funnel / example.com / Alerts).
features_h1_count="$(grep -o '<h1' "$features" | wc -l | tr -d ' ')"
test "$features_h1_count" = "1"
! grep -q '<h1[^>]*>Funnel</h1>' "$features"
! grep -q '<h1[^>]*>example.com</h1>' "$features"
! grep -q '<h1[^>]*>Alerts</h1>' "$features"

# Crawl files.
grep -q "Sitemap: https://stoma.top/sitemap.xml" "$PUBLIC/robots.txt"
grep -q "https://stoma.top/features/" "$PUBLIC/sitemap.xml"
grep -q "https://stoma.top/compare/" "$PUBLIC/sitemap.xml"
grep -q "https://stoma.top/get-started/" "$PUBLIC/sitemap.xml"

# 404 document is the not-found page, not the homepage hero.
grep -q "Page not found" "$PUBLIC/404.html"
! grep -q "Privacy-friendly web analytics you run yourself" "$PUBLIC/404.html"

# Error document is not indexable and does not claim /404/ as a canonical URL.
grep -q 'noindex' "$PUBLIC/404.html"
grep -q 'robots' "$PUBLIC/404.html"
! grep -q 'https://stoma.top/404/' "$PUBLIC/404.html"
! grep -q '/404' "$PUBLIC/sitemap.xml"

# One error document: no 404 file or 404/ directory besides root 404.html.
# A leftover 404/ is HTTP 200 at /404/. An extensionless 404 file is HTTP 200
# at /404 and does not turn the clean-URL map into a 404.
if [ -e "$PUBLIC/404" ]; then
  echo "error: 404 must not ship as a file or directory; only 404.html" >&2
  exit 1
fi

ls -la "$PUBLIC"
test -f "$PUBLIC/index.html"
test -f "$PUBLIC/CNAME"
test -f "$PUBLIC/robots.txt"
test -f "$PUBLIC/sitemap.xml"
test -f "$PUBLIC/404.html"
test -f "$PUBLIC/assets/og.png"
# PNG signature so the Pages artifact ships a real image/*, not a placeholder.
test "$(head -c 8 "$PUBLIC/assets/og.png" | od -An -tx1 | tr -d ' \n')" = "89504e470d0a1a0a"

echo "www artifact OK: $PUBLIC"
