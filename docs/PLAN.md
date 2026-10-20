# План проекта: кроссплатформенный лаунчер Minecraft

Статус: архитектура утверждена владельцем 2026-10-20. Решения — в `docs/adr/`. Контракты — в `docs/CONTRACTS.md`.

## 1. Цели

1. Все версии Minecraft (version manifest), лоадеры Forge, NeoForge, Fabric, Quilt.
2. Инстансы: создание, клонирование, импорт/экспорт `.mrpack` и CurseForge zip; моды, ресурспаки, шейдеры из Modrinth и CurseForge с зависимостями.
3. Аккаунты: оффлайн, свой Yggdrasil (authlib-injector), Ely.by. Microsoft-аккаунты не поддерживаются (решение владельца).
4. Свой сервер скинов/аккаунтов (Docker, домашний сервер рядом с mihomo); библиотека скинов с 3D-превью; скины видны на серверах с authlib-injector и в P2P.
5. P2P-мультиплеер по коду комнаты: NAT traversal + relay на нашем сервере.
6. Текстовый и голосовой чат в сессии, push-to-talk.
7. Встроенный VPN (mihomo): «только лаунчер» и «лаунчер + игра»; устойчивость к блокировкам в РФ: зеркала, ретраи, авто-переключение.
8. Слабые ПК: пресеты модов, JVM-флаги, автоподбор RAM, лёгкий режим UI.
9. Настройки: папка игры, RAM, Java, темы, RU/EN.
10. Плавный UI, который никогда не зависает; установщики `.exe`/`.msi`/AppImage/`.deb`; автообновление; токены в keyring.

## 2. Стек

| Слой | Выбор | Почему |
|---|---|---|
| Оболочка | Tauri 2.12 | маленький бинарник, Rust для тяжёлой работы (ADR-0001) |
| Фронт | React 19, TS strict, Tailwind 4, Motion, Zustand, TanStack Query, i18next, Vite | стандартный набор |
| Контракты | specta / tauri-specta `=2.0.0-rc.25` | TS-типы генерируются из Rust (ADR-0004) |
| Async | tokio | |
| HTTP | reqwest 0.13 (rustls) + свой Downloader | зеркала, хэши, прокси |
| Хранение | JSON для конфигов инстансов, SQLite (sqlx) для индексов при необходимости | переносимость инстансов |
| P2P/голос | iroh (QUIC) + iroh-relay, cpal + Opus | ADR-0002 |
| VPN | mihomo sidecar | |
| Сервер | Drasl + hub (axum) + Caddy + iroh-relay + nginx mirror-cache | ADR-0003 |
| 3D-скины | skinview3d (three.js) | |
| Лицензия | GPL-3.0-or-later | ADR-0005 |

## 3. Честные оценки и риски

| # | Риск / ограничение | Что делаем |
|---|---|---|
| R1 | **Microsoft-вход** не поддерживается (решение владельца): лицензионные аккаунты и серверы с `online-mode=true` Mojang недоступны. | Оффлайн + свой Yggdrasil; серверы — с authlib-injector на наш домен или offline-mode. |
| R2 | **TLauncher-аккаунты не поддерживаются**: закрытый API, реверс нарушает ToS и хрупок. | Оффлайн-аккаунт с тем же ником; возможно импорт скина по нику, если найдётся публичный эндпоинт (проверка в волне 0, не обещаем). Ely.by как открытая альтернатива. |
| R3 | **Скины с нашего сервера** видны только на серверах с authlib-injector (наш домен), в наших P2P-сессиях, на оффлайн-серверах со SkinsRestorer или у игроков с CustomSkinLoader. На лицензионных серверах Mojang — нет. | Честно объясняем в UI. |
| R4 | **CurseForge**: нужен API-ключ; файлы с `allowModDistribution=false` нельзя скачивать автоматически. | Заявка на ключ (владелец). `ManualDownloadRequired` → UI открывает страницу в браузере и ждёт файл в папке загрузок. |
| R5 | **Белые списки в РФ**: при строгом белом списке недоступно всё, что не в нём, включая наш сервер. | Несколько адресов подписки, BMCLAPI + своё зеркало, проверка sha1 (зеркалам не доверяем), ретраи, `Route::Auto`. Гарантий нет. |
| R6 | **«Лаунчер + игра»**: Netty в Minecraft игнорирует `-DsocksProxyHost`. | TUN-режим mihomo с правилами по процессу (наш Java). Права: служба/UAC на Windows, `setcap cap_net_admin` через pkexec на Linux. |
| R7 | **Push-to-talk на Wayland**: глобальный перехват клавиш запрещён. | XDG GlobalShortcuts portal (KDE, новые GNOME) или VAD-режим. Windows/X11 — low-level hook без перехвата клавиши. |
| R8 | **Смешанные аккаунты в P2P** (MS + Yggdrasil + оффлайн). | Режим комнаты `yggdrasil` (хост с authlib-injector → Drasl с fallback на Mojang) или `open` (без проверки, у оффлайн нет скинов). PoC в волне 0. |
| R9 | **Домашний сервер за NAT/CGNAT**. | Нужен белый IP или проброс 443/tcp+udp и 3478/udp; иначе VPS под relay. |
| R10 | **Keyring**: лимит ~2.5 КБ на запись в Windows; на минимальном Linux может не быть Secret Service. | Храним только refresh-токены; fallback — зашифрованный файл с предупреждением. |
| R11 | **WebKitGTK** медленнее Chromium. | Лёгкий режим (без blur, reduced motion), авто-включение на слабом железе. |
| R12 | **RC-версии specta**. | Точные пины, обновляет только lead. |

## 4. Модули и границы

```
crates/
  lc-types      [lead]     DTO, AppError, ID, payload событий (всё экспортируется в TS)
  lc-runtime    [lead]     ProgressSink, CancelToken, Paths, SettingsStore, LaunchPlan, LaunchContributor
  lc-http       [net]      Downloader: зеркала, ретраи, sha1, Route Direct/Proxied/Auto
  lc-core       [core]     version manifest, Java manager, инстансы, запуск JVM, логи, SystemProbe
  lc-loaders    [mods]     Fabric/Quilt (meta API), Forge/NeoForge (install_profile + processors)
  lc-content    [mods]     Modrinth/CurseForge, разрешение зависимостей, mrpack/CF zip, пресеты
  lc-accounts   [accounts] оффлайн, Yggdrasil/authlib-injector, keyring, скины
  lc-vpn        [net]      mihomo: подписки, режимы, TUN, health
  lc-p2p        [net]      iroh: комнаты, TCP-туннель, LAN-детект, чат; proto.rs — протокол
  lc-voice      [voice]    cpal, Opus, jitter buffer, PTT/VAD, шумоподавление
apps/desktop/
  src-tauri/    [lead]     wiring (state, tasks, events, observers, bindings); commands/<domain>.rs — по ролям
  src/          [ui]       весь фронтенд; bindings.ts — автоген (lead)
server/
  hub/          [server-ops] axum: комнаты, зеркала, библиотека скинов
  openapi/      [lead]     контракт hub + описание Yggdrasil
  docker-compose.yml, deploy/  [server-ops]
tests/          [qa]
.github/        [release]
```

Граф зависимостей (только на `contract.rs` чужих крейтов):

```
lc-types ← lc-runtime ← lc-http ← { lc-accounts, lc-loaders, lc-content }
lc-core → lc-accounts (AuthProvider), lc-loaders (LoaderInstaller)
lc-voice → lc-p2p (DatagramTransport)
src-tauri → всё (единственное место, где стабы заменяются реализациями)
```

### Ключевые механики

- **Задачи**: всё дольше 100 мс → `spawn_task` → сразу `TaskId`, прогресс в `Channel<TaskEvent>`, глобальный `tasks://changed`. Отмена через `CancelToken`.
- **Загрузки**: общие `libraries/` и `assets/` для всех инстансов; запись в `*.part` + rename; ограничение параллельности по хостам; рейтинг зеркал; `Blocked` → `net://blocked` и прокси.
- **Java**: runtime-манифест Mojang (jre-legacy, gamma, delta…), fallback Adoptium; выбор по `javaVersion` из version json.
- **Forge/NeoForge**: парсим `install_profile.json` и выполняем processors сами.
- **Запуск**: `lc-core` строит `LaunchPlan`; `LaunchContributor`-ы в порядке accounts → mods → vpn → p2p.
- **RAM**: стартовая формула `clamp(total × 0.4, 2 ГБ, 8 ГБ)`, нижняя граница 4 ГБ для крупных модпаков; мало свободной RAM → предложить пресет `potato`.
- **Пресеты**: Fabric — Sodium, Lithium, FerriteCore, ModernFix, EntityCulling, ImmediatelyFast; Forge/NeoForge — Embeddium и аналоги (по версии).
- **P2P**: хост открывает мир в LAN → лаунчер ловит LAN-анонс своей игры (224.0.2.60:4445) → порт. Гость: локальный порт + `--quickPlayMultiplayer` (1.20+) или `--server/--port`, плюс локальный LAN-анонс.

## 5. Роли

| Роль | Зона |
|---|---|
| lead | контракты, wiring, ADR, мерж в main |
| core | `lc-core`, команды versions/java/instances/system |
| mods | `lc-loaders`, `lc-content`, команды loaders/content |
| accounts | `lc-accounts`, команды accounts/skins |
| net | `lc-http`, `lc-vpn`, `lc-p2p`, команды vpn/rooms, sidecar mihomo |
| voice | `lc-voice`, команды voice |
| ui | `apps/desktop/src` (кроме `bindings.ts`) |
| server-ops | `server/` (кроме `openapi/`), единственный с SSH |
| qa | `tests/`, ревью, запуск тестов; прод-код не пишет |
| release | CI, `tauri.conf.json` (bundle/updater), подпись, установщики, `commands/app.rs` |

Точные папки и правила — в `CLAUDE.md` и `.claude/agents/<роль>.md`.

## 6. Волны

### Волна 0 — фундамент и PoC
- [x] lead: workspace, контракты, стабы, bindings, документы, правила агентов (этот коммит).
- [ ] **Владелец**: заявка Azure + Mojang (R1), ключ CurseForge (R4), домен/белый IP (R9), правила безопасности сервера для server-ops.
- [ ] net: PoC iroh hole punching с российских провайдеров и мобильного интернета.
- [ ] net: PoC mihomo TUN с правилами по процессу на Windows и Linux.
- [ ] accounts + net: PoC LAN-аутентификации со смешанными аккаунтами + Drasl fallback (R8).
- [ ] ui: PoC производительности Motion на WebKitGTK.
- [ ] server-ops: Caddy + Drasl + hub-заглушка + iroh-relay + mirror-cache, проверка доступности.
- [ ] release: CI на win/linux зелёный, ключ подписи updater, пустой установщик.

### Волна 1 — параллельно, зависимости только на контракты
- net: `lc-http` (**блокирует остальных** — первым делом), затем VPN «только лаунчер».
- core: манифест, Java manager, assets/libraries, ванильный запуск (оффлайн-аккаунт).
- accounts: оффлайн-аккаунты, keyring (для токенов Yggdrasil); Yggdrasil + authlib-injector — когда поднимется сервер.
- ui: оболочка, дизайн-система, темы, i18n, все экраны на моках.
- server-ops: рабочий Drasl, hub `/health`, `/mirrors`, `/skins`.
- qa: тест-план, фикстуры, contract-тесты (bindings, OpenAPI).
- release: матрица сборок, артефакты установщиков.

### Волна 2 — нужны запуск из core и lc-http
- mods: Fabric/Quilt → Forge/NeoForge; Modrinth с зависимостями.
- core: инстансы (клон, ремонт), логи, крэш-детект, SystemProbe.
- net: lc-p2p (туннель, комнаты через hub).
- voice: аудио-пайплайн отдельно (loopback-тест).
- accounts: библиотека скинов, применение через hub.
- ui: реальные биндинги, 3D-превью скинов, центр задач.

### Волна 3 — интеграция
- mods: CurseForge, импорт/экспорт mrpack/CF zip, пресеты оптимизации.
- net: VPN «лаунчер + игра» (TUN, права).
- net + core: P2P вместе с запуском (`LaunchContributor`).
- voice: голос и чат в комнатах, PTT.
- server-ops: комнаты и WS hub в проде, мониторинг.

### Волна 4 — укрепление и релиз
- qa: тесты в РФ-сетях (блокировки, зеркала, VPN).
- ui: лёгкий режим, полнота RU/EN.
- release: NSIS/MSI/AppImage/deb, автообновление через зеркала, подпись.
- закрытая бета.

**Критический путь:** `lc-types` → `lc-http` → запуск в core → лоадеры → P2P + запуск → голос.

## 7. Решения владельца и открытые вопросы

Решено (2026-10-20):
- Название **MineUP**, identifier `ru.dedseckms.mineup`, домен сервера `dedseckms.ru`.
- **Microsoft-аккаунты не нужны** — убраны из контрактов. Заявки Azure/Mojang и CurseForge не подаются; контент — только Modrinth, CurseForge отложен.
- Правила безопасности сервера — в `.claude/agents/server-ops.md`.

Открыто:
1. Белый IP / проброс портов (по итогам разведки server-ops).
2. Политика регистрации на сервере скинов (открытая / по инвайтам).
