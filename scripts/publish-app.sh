#!/usr/bin/env bash
# Публикация установщика релизом на GitHub: кнопка «Обновить» в программе
# скачивает этот установщик, проверяет SHA-256 и запускает его.
#
# Сначала: сборка установщика (tauri build) и gh auth login.
# Запуск: scripts/publish-app.sh [версия]   (по умолчанию — версия tauri.conf.json)
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
VER="${1:-$(awk -F'"' '/"version"/{print $4; exit}' "$HERE/src-tauri/tauri.conf.json")}"
REPO="${APP_REPO:-CrazyCoffee3663/RDPMgrCoffee}"
EXE="$HERE/src-tauri/target/release/bundle/nsis/RDPMgrCoffee_${VER}_x64-setup.exe"
command -v gh >/dev/null 2>&1 || { echo "gh не найден в PATH"; exit 1; }
[ -f "$EXE" ] || { echo "Нет установщика: $EXE"; exit 1; }

sha256sum "$EXE" | awk '{print $1}' > "$EXE.sha256"
if gh release view "v$VER" --repo "$REPO" >/dev/null 2>&1; then
  gh release upload "v$VER" "$EXE" "$EXE.sha256" --clobber --repo "$REPO"
else
  gh release create "v$VER" "$EXE" "$EXE.sha256" --repo "$REPO" \
    --title "RDPMgrCoffee $VER (Windows x64)" \
    --notes "Установщик RDPMgrCoffee $VER. Программа раз в сутки проверяет этот релиз и обновляется одной кнопкой (SHA-256)."
fi
echo "Опубликовано: https://github.com/$REPO/releases/tag/v$VER"
