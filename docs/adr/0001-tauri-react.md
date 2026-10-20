# ADR-0001: Tauri 2 + React/TypeScript

Статус: принято (2026-10-20)

**Контекст.** Нужен кроссплатформенный (Windows + Linux) лаунчер с плавным UI, тяжёлыми загрузками и запуском JVM, маленьким установщиком.

**Решение.** Tauri 2 (Rust backend) + React 19 + TypeScript strict + Tailwind 4 + Motion + Zustand + TanStack Query + i18next. Вся тяжёлая работа — в Rust (tokio); UI получает прогресс через `tauri::ipc::Channel`.

**Последствия.**
- Установщик ~10 МБ вместо 100+ МБ у Electron.
- На Linux WebView = WebKitGTK: медленнее рендер, нет надёжного WebRTC → лёгкий режим UI, без `backdrop-filter`, голос не в WebView (ADR-0002).
- Unit-тесты Tauri-крейта на Windows требуют встроенного манифеста Common Controls v6 (`apps/desktop/src-tauri/build.rs`).
