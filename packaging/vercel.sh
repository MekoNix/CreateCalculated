#!/bin/sh
# Builds the site as Vercel serves it and uploads it: a test version, or the real one with --prod.
# The very first upload of a new project goes live whatever the flag says, later ones follow it.
# Recipes and pictures must already be in data/. Only the finished output in data/vercel is
# uploaded: no source code or game JAR, but the site does publish the recipes and pictures.
set -eu

cd "$(dirname "$0")/.."
target=x86_64-unknown-linux-musl
site=data/vercel
out=$site/.vercel/output

for file in data/bundle.json data/icons/index.json; do
    if [ ! -f "$file" ]; then
        echo "$file was not found, run make data first" >&2
        exit 1
    fi
done

rustup target add "$target" >/dev/null
# Without the flag the program would carry the builder's home folder in its panic messages.
RUSTFLAGS="--remap-path-prefix=$HOME=/build" cargo build --release -p createcalc-server --features vercel --bin createcalc-vercel --target "$target"

rm -rf "$out"
mkdir -p "$out/static/api" "$out/functions/api.func"
cp server/page.html "$out/static/index.html"
cp -r data/icons "$out/static/api/icons"
rm -f "$out/static/api/icons/.no-install-prompt"
cp "target/$target/release/createcalc-vercel" "$out/functions/api.func/executable"
strip "$out/functions/api.func/executable"
cat >"$out/functions/api.func/.vc-config.json" <<'JSON'
{"handler":"executable","runtime":"executable","architecture":"x86_64","supportsResponseStreaming":true,"runtimeLanguage":"rust"}
JSON
# Pictures have the index version in their address, so they can be kept for good.
cat >"$out/config.json" <<'JSON'
{
  "version": 3,
  "routes": [
    {"src": "/api/icons/.+\\.png", "headers": {"cache-control": "public, max-age=31536000, immutable"}, "continue": true},
    {"handle": "filesystem"},
    {"src": "/api(/.*)?", "dest": "/api"}
  ]
}
JSON

if [ ! -f "$site/.vercel/project.json" ]; then
    npx --yes vercel link --yes --project createcalculated --cwd "$site"
fi
npx --yes vercel deploy --prebuilt --yes --cwd "$site" "$@"
