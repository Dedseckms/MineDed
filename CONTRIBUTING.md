# Как помочь проекту

Спасибо, что хотите улучшить MineDed! Перед участием прочитайте [правила сообщества](CODE_OF_CONDUCT.md).

## Сообщить об ошибке или предложить идею

- Ошибка → [issue «Ошибка»](https://github.com/Dedseckms/MineDed/issues/new?template=bug_report.yml).
  Приложите версию лаунчера, ОС и лог: лог игры — «Играть → Лог игры», лог лаунчера — `%LOCALAPPDATA%\ru.dedseckms.mineup\logs` (Windows) или `~/.local/share/ru.dedseckms.mineup/logs` (Linux).
- Идея → [issue «Предложение»](https://github.com/Dedseckms/MineDed/issues/new?template=feature_request.yml).
- Уязвимость → **не** в issues, а по инструкции из [SECURITY.md](SECURITY.md).

## Сборка из исходников

Нужны Rust (stable), Node 20+ и pnpm. На Linux также:
`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libx11-dev libxdo-dev pkg-config`.

```sh
pnpm install
pnpm --filter desktop build      # фронтенд (нужен для сборки Tauri)
pnpm dev                         # запустить лаунчер в режиме разработки
```

## Перед pull request

Всё должно проходить:

```sh
pnpm --filter desktop typecheck
pnpm --filter desktop test
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

На машине с 16 ГБ памяти добавляйте `-j 2` к командам cargo.

## Стиль

- **Rust:** `cargo fmt`, без предупреждений clippy, без `unwrap`/`expect` вне тестов, без `unsafe`.
  Ошибки — через `lc_types::AppError`. Комментарии — на английском, по делу.
- **TypeScript:** strict, без `any`; все строки интерфейса — ключи i18n в `ru.json` и `en.json`.
  `apps/desktop/src/bindings.ts` генерируется (`cargo test -p desktop export_bindings`), руками не правится.
- **Коммиты:** `<область>(<часть>): <что сделано>` по-английски, например `ui(skins): show 3D cape preview`.
- Новая логика — с тестами. Один PR — одна задача.

## Лицензия

Отправляя код, вы соглашаетесь распространять его на условиях
[GPL-3.0-or-later](LICENSE). Заимствованный код — только из GPL-совместимых источников с указанием авторства.
