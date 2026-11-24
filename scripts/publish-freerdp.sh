#!/usr/bin/env bash
# Публикация собранного FreeRDP релизом на GitHub: кнопка «Обновить» в программе
# скачивает этот архив (~8 МБ), проверяет SHA-256 и заменяет встроенный FreeRDP.
#
# Сначала: scripts/build-freerdp.sh <версия> (из MSYS2 UCRT64) и gh auth login.
# Запуск: scripts/publish-freerdp.sh [версия]   (по умолчанию — version.txt сборки)
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$HERE/src-tauri/resources/freerdp"
VER="${1:-$(cat "$OUT/version.txt")}"
REPO="${FREERDP_REPO:-CrazyCoffee3663/RDPMgrCoffee}"
WORK="${WORK:-$HERE/../freerdp-build}"
ZIP="$WORK/freerdp-$VER-win-x64.zip"
command -v gh >/dev/null 2>&1 || { echo "gh не найден в PATH"; exit 1; }
[ -f "$OUT/wfreerdp.exe" ] || { echo "Нет сборки: $OUT"; exit 1; }

rm -f "$ZIP" "$ZIP.sha256"
# Архив с плоским корнем: wfreerdp.exe и DLL рядом, ossl-modules/ подкаталогом
(cd "$OUT" && "/c/Program Files/7-Zip/7z.exe" a -tzip "$(cygpath -w "$ZIP")" ./* >/dev/null)
sha256sum "$ZIP" | awk '{print $1}' > "$ZIP.sha256"

if gh release view "freerdp-$VER" --repo "$REPO" >/dev/null 2>&1; then
  gh release upload "freerdp-$VER" "$ZIP" "$ZIP.sha256" --clobber --repo "$REPO"
else
  gh release create "freerdp-$VER" "$ZIP" "$ZIP.sha256" --repo "$REPO" \
    --title "FreeRDP $VER (Windows x64)" \
    --notes "Облегчённая сборка FreeRDP $VER для RDPMgrCoffee (scripts/build-freerdp.sh $VER): wfreerdp.exe и только нужные DLL, провайдер OpenSSL legacy для NTLM."
fi
echo "Опубликовано: https://github.com/$REPO/releases/tag/freerdp-$VER"
