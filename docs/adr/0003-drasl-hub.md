# ADR-0003: Drasl для Yggdrasil + собственный `hub`

Статус: принято (2026-10-20)

**Контекст.** Нужен свой сервер аккаунтов и скинов, совместимый с authlib-injector, со скинами, видимыми на серверах и в P2P.

**Решение.**
- **Drasl** (Go, Docker) — Yggdrasil API, подпись текстур, fallback на API Mojang (MS-игроки проходят проверку у хоста P2P и видны со скинами).
- **hub** (Rust/axum, `server/hub`) — комнаты, список зеркал, библиотека скинов поверх Drasl admin API. Контракт: `server/openapi/hub.yaml`.
- Caddy (TLS), `iroh-relay`, nginx mirror-cache — в одном `docker-compose.yml`.

**Последствия.** Не пишем и не поддерживаем свою криптографию Yggdrasil. Если Drasl перестанет устраивать — переписываем на axum за тем же HTTP-контрактом (`server/openapi/yggdrasil.md`).
