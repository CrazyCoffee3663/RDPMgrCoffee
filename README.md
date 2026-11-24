# RDPMgrCoffee

Менеджер подключений: RDP (встроенный FreeRDP), SSH, Telnet, COM-порт, VNC, веб-вкладки и WinBox.
Пароли — в вашем хранилище Bitwarden/Vaultwarden или в локальном зашифрованном хранилище;
шифрование совместимо с Bitwarden (EncString AES-256-CBC + HMAC-SHA256, PBKDF2/Argon2id, RSA-OAEP).

Tauri 2 + Svelte 5 (интерфейс), Rust (бэкенд). Windows 10/11 x64.

## Сборка

Нужны:

- Node.js 20+ и npm
- Rust (stable)
- MSYS2 с тулчейном UCRT64 (для сборки FreeRDP и aws-lc-rs)

FreeRDP в репозитории нет — сначала соберите его:

    # в оболочке MSYS2 UCRT64
    pacman -S mingw-w64-ucrt-x86_64-freerdp mingw-w64-ucrt-x86_64-toolchain
    scripts/build-freerdp.sh

Скрипт кладёт `wfreerdp.exe` и его DLL в `src-tauri/resources/freerdp/`.
Без этой папки не пройдут ни `cargo check`, ни `npm run tauri build`:
tauri-build проверяет наличие ресурсов комплекта (bundle resources) при компиляции.

Обновление FreeRDP у пользователей — одной кнопкой (Настройки → FreeRDP → «Обновить»,
или уведомление в строке состояния): программа скачивает архив сборки с GitHub-релиза
репозитория из константы `FREERDP_REPO` в `src-tauri/src/tools.rs`, проверяет SHA-256,
опробует запуск и заменяет каталог с откатом. GitHub API не используется — только
github.com: api.github.com блокируют hosts-файлы некоторых антивирусов.

Публикация новой сборки мейнтейнером (нужен `gh auth login`):

    scripts/build-freerdp.sh 3.33.1     # в оболочке MSYS2 UCRT64
    scripts/publish-freerdp.sh 3.33.1   # релиз freerdp-3.33.1 с архивом и SHA-256

Затем:

    npm install
    npm run tauri dev       # запуск для разработки
    npm run tauri build     # установщик NSIS в src-tauri/target/release/bundle/nsis

## Проверки

    npm run check           # svelte-check (типизация интерфейса)
    cargo test --manifest-path src-tauri/Cargo.toml

## Рекомендуемая среда

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).
