#!/usr/bin/env bash
# Construit NETIV pour le navigateur : module WebAssembly + interface web.
#
#   crates/netiv-wasm/build-web.sh [dossier de sortie]
#
# Résultat (par défaut crates/netiv-wasm/dist/) : un dossier statique autonome,
# à servir tel quel (démo : `python3 -m http.server -d crates/netiv-wasm/dist`)
# ou à copier dans une plateforme (ITEtude : public/netiv/<version>/).
#
# Outils : rustup target wasm32-unknown-unknown, wasm-pack, wasm-opt (binaryen).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="${1:-$here/dist}"
web="$root/crates/netiv-api/web"
pkg="$here/pkg"

for tool in wasm-pack wasm-opt cargo; do
  command -v "$tool" >/dev/null || { echo "outil manquant : $tool" >&2; exit 1; }
done

echo "==> Compilation WebAssembly (profil wasm-release)"
wasm-pack build "$here" --target web --profile wasm-release --no-opt --no-pack --out-dir "$pkg"

rm -rf "$out"
mkdir -p "$out"

echo "==> Optimisation (wasm-opt -Oz)"
wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
  --enable-mutable-globals "$pkg/netiv_wasm_bg.wasm" -o "$out/netiv_wasm_bg.wasm"

echo "==> Interface web"
cp "$pkg/netiv_wasm.js" "$pkg/netiv_wasm.d.ts" "$out/"
cp "$web/app.js" "$web/style.css" "$web/favicon.svg" "$here/web/netiv-boot.js" "$out/"
# Même page que `netiv serve` ; seul le démarrage change : le moteur
# WebAssembly est chargé avant l'interface.
grep -q '<script src="app.js" defer></script>' "$web/index.html" \
  || { echo "index.html : balise <script src=\"app.js\"> introuvable" >&2; exit 1; }
sed 's|<script src="app.js" defer></script>|<script type="module" src="netiv-boot.js"></script>|' \
  "$web/index.html" > "$out/index.html"

echo "==> Catalogue des épreuves"
cargo run --quiet -p netiv-wasm --example catalog > "$out/catalog.json"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
raw=$(wc -c < "$out/netiv_wasm_bg.wasm")
gz=$(gzip -9 -c "$out/netiv_wasm_bg.wasm" | wc -c)
br=$(node -e "const z=require('zlib'),fs=require('fs');process.stdout.write(String(z.brotliCompressSync(fs.readFileSync(process.argv[1]),{params:{[z.constants.BROTLI_PARAM_QUALITY]:11}}).length))" "$out/netiv_wasm_bg.wasm" 2>/dev/null || echo null)
total_gz=0
for f in "$out"/*.wasm "$out"/*.js "$out"/*.css "$out"/index.html; do
  total_gz=$((total_gz + $(gzip -9 -c "$f" | wc -c)))
done

# Fiche du paquet : version, tailles (octets) et empreintes, pour la mise à
# jour et l'affichage du poids à télécharger.
{
  echo "{"
  echo "  \"name\": \"netiv-wasm\","
  echo "  \"version\": \"$version\","
  echo "  \"wasm\": { \"bytes\": $raw, \"gzip\": $gz, \"brotli\": $br },"
  echo "  \"download_gzip\": $total_gz,"
  echo "  \"files\": {"
  first=1
  for f in $(cd "$out" && ls | grep -v '^manifest.json$' | sort); do
    [ $first -eq 1 ] || echo ","
    first=0
    printf '    "%s": { "bytes": %s, "sha256": "%s" }' "$f" "$(wc -c < "$out/$f")" \
      "$(sha256sum "$out/$f" | cut -d' ' -f1)"
  done
  echo ""
  echo "  }"
  echo "}"
} > "$out/manifest.json"

echo
echo "NETIV $version pour le navigateur : $out"
printf '  module .wasm : %s octets, %s gzip, %s brotli\n' "$raw" "$gz" "$br"
printf '  téléchargement total (gzip) : %s octets\n' "$total_gz"
