# Tauri + SvelteKit + TypeScript

This template should help get you started developing with Tauri, SvelteKit and TypeScript in Vite.

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).



# connmgr

Менеджер подключений (Tauri 2, Svelte). Пароли хранятся в Bitwarden/Vaultwarden.

## Внешние файлы (не в репозитории)

Нужно положить в src-tauri/resources:

- bw/bw.exe : Bitwarden CLI 2026.9.1 (Windows, zip с сайта Bitwarden)
- freerdp/ : wfreerdp.exe 3.32.0 и все его DLL из MSYS2 UCRT64
  (pacman -S mingw-w64-ucrt-x86_64-freerdp, затем скопировать wfreerdp.exe и
  библиотеки, найденные командой ldd, см. историю проекта)
- putty/putty.exe : PuTTY (64-bit x86), отдельный exe с официального сайта

Резервная копия готового комплекта FreeRDP: C:\dev\freerdp-bundle

## Сборка

npm install
npm run tauri dev       # запуск для разработки
npm run tauri build     # установщик в src-tauri/target/release/bundle/nsis