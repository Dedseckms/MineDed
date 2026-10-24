# ADR-0004: Контракты в коде, TS-типы генерируются из Rust

Статус: принято (2026-10-20)

**Контекст.** Несколько модулей разрабатываются параллельно; расхождение типов Rust↔TS и между крейтами — главный риск интеграции.

**Решение.**
- Все DTO — в `lc-types` (serde camelCase + `specta::Type`).
- Межкрейтовые трейты — в `crates/<crate>/src/contract.rs`; реализации — в остальных файлах крейта.
- Все команды и события регистрируются в `apps/desktop/src-tauri/src/bindings.rs`; `cargo test -p desktop export_bindings` генерирует `apps/desktop/src/bindings.ts`; CI падает при расхождении.
- Каждый трейт имеет `Stub*`-реализацию, возвращающую `notImplemented`, поэтому проект всегда собирается.
- Фронтенд вызывает бэкенд только через `src/api/client.ts` (реальные биндинги или моки).
- Контракты меняет только lead.

**Последствия.** tauri-specta/specta пока в RC → версии закреплены точно (`=2.0.0-rc.25`). `serde_json::Value` в DTO запрещён (specta не может его экспортировать) — `AppError.details` это `Record<string,string>`. u64 экспортируются как `number` (`dangerously_cast_bigints_to_number`), значения < 2^53.
