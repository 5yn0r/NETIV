#!/usr/bin/env bash
# Vérifie qu'aucun secret n'est suivi par git : clés d'API, clés privées,
# comptes de service, jetons, fichiers d'environnement (sauf .env.example).
#
#   scripts/check-secrets.sh        (à lancer avant chaque push ; aussi en CI)
#
# N'affiche jamais une valeur trouvée : seulement le fichier, la ligne et le
# type de secret. Code de sortie 1 si quelque chose est trouvé.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

found=0
report() { printf '  %s\n' "$1"; found=1; }

# Motifs de secrets connus (type|expression régulière étendue).
patterns=(
  "clé Google (Firebase, Gemini…)|AIza[0-9A-Za-z_-]{30,}"
  "clé OpenAI|sk-(proj-)?[A-Za-z0-9_-]{32,}"
  "clé Anthropic|sk-ant-[A-Za-z0-9_-]{20,}"
  "jeton GitHub|(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}"
  "clé AWS|AKIA[0-9A-Z]{16}"
  "jeton Slack|xox[abprs]-[A-Za-z0-9-]{10,}"
  "clé privée|-----BEGIN ([A-Z]+ )?PRIVATE KEY-----"
  "compte de service Google|\"private_[k]ey\"[[:space:]]*:"
  "jeton JWT|eyJ[A-Za-z0-9_-]{20,}\.eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}"
)

echo "Recherche de secrets dans les fichiers suivis…"
for entry in "${patterns[@]}"; do
  label="${entry%%|*}"
  regex="${entry#*|}"
  # -I : ignore les binaires ; les verrous de dépendances sont exclus (empreintes).
  while IFS=: read -r file line _; do
    [ -n "$file" ] && report "$file:$line : $label"
  done < <(git grep -nIE "$regex" -- . ':!*.lock' ':!package-lock.json' ':!scripts/check-secrets.sh' 2>/dev/null || true)
done

echo "Recherche de fichiers sensibles suivis…"
while read -r file; do
  [ -n "$file" ] && report "$file : fichier sensible suivi par git"
done < <(git ls-files | grep -E '(^|/)\.env($|\.)|(^|/)(service-?account|serviceAccount)[^/]*\.json$|\.(pem|key|p12|pfx)$' | grep -vE '(^|/)\.env\.example$' || true)

if [ "$found" -ne 0 ]; then
  echo "Secrets trouvés : retirez-les avant de pousser (et révoquez toute clé déjà poussée)." >&2
  exit 1
fi
echo "Aucun secret trouvé."
