#!/usr/bin/env bash
# Сборка FreeRDP для RDPMgrCoffee без FFmpeg (видео H.264 декодирует Windows Media Foundation).
#
# Запуск из оболочки MSYS2 UCRT64 (или: C:\msys64\usr\bin\bash -lc "MSYSTEM=UCRT64 ... build-freerdp.sh"):
#   scripts/build-freerdp.sh 3.32.0
# Нужны пакеты: mingw-w64-ucrt-x86_64-{gcc,cmake,ninja,pkgconf,openssl,zlib,cjson,uriparser}
#
# Результат: src-tauri/resources/freerdp (wfreerdp.exe и только нужные DLL).
# Исходники проверяются по SHA-256, опубликованному в выпуске FreeRDP на GitHub.
set -euo pipefail

VER="${1:-3.32.0}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${WORK:-$HERE/../freerdp-build}"
OUT="$HERE/src-tauri/resources/freerdp"
SRC="$WORK/src/freerdp-$VER"
BUILD="$WORK/build-$VER"
PREFIX="$WORK/install-$VER"
URL="https://github.com/FreeRDP/FreeRDP/releases/download/$VER/freerdp-$VER.tar.gz"

export PATH="/ucrt64/bin:$PATH"
# Без этой переменной CMake не определяет процессор и отключает флаги SSE/AVX
export PROCESSOR_ARCHITECTURE="${PROCESSOR_ARCHITECTURE:-AMD64}"
mkdir -p "$WORK/src"
cd "$WORK/src"
if [ ! -d "$SRC" ]; then
  curl -sSLf -o "freerdp-$VER.tar.gz" "$URL"
  curl -sSLf -o "freerdp-$VER.tar.gz.sha256" "$URL.sha256"
  sha256sum -c "freerdp-$VER.tar.gz.sha256"
  tar xzf "freerdp-$VER.tar.gz"
fi
# Исправления из рецепта MSYS2 (scripts/freerdp-patches), один раз
if [ ! -f "$SRC/.rdpmgr-patched" ]; then
  for p in "$HERE"/scripts/freerdp-patches/*.patch; do
    [ -f "$p" ] && patch -d "$SRC" -p1 --forward < "$p"
  done
  touch "$SRC/.rdpmgr-patched"
fi

cmake -S "$SRC" -B "$BUILD" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DBUILD_TESTING=OFF \
  -DCMAKE_C_FLAGS="-D__STDC_NO_THREADS__=1 -Wno-incompatible-pointer-types" \
  -DUSE_UNWIND=OFF -DWITH_WIN_CONSOLE=ON \
  -DWITH_CLIENT_WINDOWS=ON -DWITH_CLIENT_SDL=OFF -DWITH_CLIENT_SDL3=OFF \
  -DWITH_SERVER=OFF -DWITH_SHADOW=OFF -DWITH_PROXY=OFF -DWITH_SAMPLE=OFF \
  -DWITH_WINPR_TOOLS=ON -DWITH_WINPR_TOOLS_CLI=OFF -DWITH_MANPAGES=OFF -DWITH_DOCUMENTATION=OFF \
  -DWITH_FFMPEG=OFF -DWITH_DSP_FFMPEG=OFF -DWITH_VIDEO_FFMPEG=OFF -DWITH_SWSCALE=OFF \
  -DWITH_MEDIA_FOUNDATION=ON -DWITH_OPENH264=OFF -DWITH_GFX_AV1=OFF \
  -DWITH_CAIRO=OFF -DWITH_JPEG=OFF -DWITH_WEBP=OFF -DWITH_OPUS=OFF -DWITH_FDK_AAC=OFF \
  -DWITH_KRB5=OFF -DWITH_FUSE=OFF -DWITH_AAD=OFF -DWITH_WEBVIEW_AAD_AUTH_HELPER=OFF \
  -DCHANNEL_URBDRC=OFF \
  -DWITH_INTERNAL_MD4=ON -DWITH_INTERNAL_RC4=ON -DWITH_INTERNAL_MD5=ON \
  -DWITH_WINDOWS_CERT_STORE=ON \
  -DWITH_URIPARSER=ON -DWITH_CJSON=ON \
  -DWITH_SIMD=ON
cmake --build "$BUILD" -j "$(nproc)"
cmake --install "$BUILD"

# Собрать wfreerdp.exe и его зависимости (только DLL из /ucrt64/bin и установки FreeRDP)
rm -rf "$OUT.new" && mkdir -p "$OUT.new"
cp "$PREFIX/bin/wfreerdp.exe" "$OUT.new/"
seen=""
queue="$OUT.new/wfreerdp.exe"
while [ -n "$queue" ]; do
  f="${queue%%$'\n'*}"; [ "$queue" = "$f" ] && queue="" || queue="${queue#*$'\n'}"
  for dll in $(objdump -p "$f" | awk '/DLL Name:/ {print $3}'); do
    case " $seen " in *" ${dll,,} "*) continue ;; esac
    seen="$seen ${dll,,}"
    for dir in "$PREFIX/bin" /ucrt64/bin; do
      if [ -f "$dir/$dll" ]; then
        cp "$dir/$dll" "$OUT.new/"
        queue="$queue"$'\n'"$OUT.new/$dll"
        break
      fi
    done
  done
  queue="${queue#$'\n'}"
done
# Плагины каналов FreeRDP (если собраны отдельными DLL)
if [ -d "$PREFIX/lib/freerdp3" ]; then cp -r "$PREFIX/lib/freerdp3" "$OUT.new/"; fi
# Версия — для проверки обновлений в программе
echo "$VER" > "$OUT.new/version.txt"
rm -rf "$OUT.old"; [ -d "$OUT" ] && mv "$OUT" "$OUT.old"
mv "$OUT.new" "$OUT"
echo "Готово: $OUT ($(du -sh "$OUT" | cut -f1)), прежняя версия — $OUT.old"
