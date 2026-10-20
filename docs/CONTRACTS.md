# Контракты

Владелец всех контрактов — **lead**. Субагенты их не редактируют: если нужно изменение, агент останавливается и пишет запрос (формат в конце файла). Обоснование подхода — ADR-0004.

## 1. Где что лежит

| Контракт | Файл(ы) | Потребители |
|---|---|---|
| DTO, ID, `AppError`, `TaskEvent`, payload событий | `crates/lc-types/src/*.rs` | все крейты, фронт (через bindings) |
| Прогресс, отмена, пути, настройки, план запуска | `crates/lc-runtime/src/*.rs` | все backend-крейты |
| `Downloader`, `Route`, `DownloadReq` | `crates/lc-http/src/contract.rs` | core, mods, accounts, vpn |
| `VersionService`, `JavaManager`, `InstanceService`, `GameLauncher`, `GameObserver`, `SystemProbe` | `crates/lc-core/src/contract.rs` | src-tauri |
| `LoaderInstaller`, `InstallCtx`, `LoaderRegistry` | `crates/lc-loaders/src/contract.rs` | core, src-tauri |
| `ContentSource`, `ContentService`, `PackIo`, `PresetService` | `crates/lc-content/src/contract.rs` | src-tauri |
| `AuthProvider`, `LaunchAuth`, `AccountService`, `TokenStore`, `SkinService`, `Secret` | `crates/lc-accounts/src/contract.rs` | core, src-tauri |
| `VpnController` | `crates/lc-vpn/src/contract.rs` | src-tauri, lc-http (proxy URL) |
| `RoomService`, `DatagramTransport`, `RoomObserver` | `crates/lc-p2p/src/contract.rs` | src-tauri, voice |
| P2P wire protocol (`ALPN`, `ControlMsg`, `StreamOpen`, `VoiceHeader`) | `crates/lc-p2p/src/proto.rs` | net, voice |
| `VoiceEngine`, `VoiceObserver` | `crates/lc-voice/src/contract.rs` | src-tauri |
| Реестр команд и событий | `apps/desktop/src-tauri/src/bindings.rs`, `events.rs` | фронт |
| Сервисный контейнер, задачи, наблюдатели | `apps/desktop/src-tauri/src/{state,tasks,observers}.rs` | — |
| TS-типы и обёртки команд (**автоген**) | `apps/desktop/src/bindings.ts` | ui |
| HTTP API hub | `server/openapi/hub.yaml` | hub, lc-p2p, lc-http, lc-accounts |
| Yggdrasil (Drasl) | `server/openapi/yggdrasil.md` | lc-accounts, server-ops |

Каждый трейт имеет `Stub*`-реализацию в `src/stub.rs` того же крейта; стаб возвращает `ErrorCode::NotImplemented` (или пустой список/«выключено»). Замена стаба реальной реализацией делается **только** в `apps/desktop/src-tauri/src/state.rs` (lead).

## 2. Общие правила

- На проводе всё в **camelCase** (`#[serde(rename_all = "camelCase")]`), enum'ы — строки в camelCase.
- Ошибка через IPC/между крейтами — только `AppError { code: ErrorCode, message, details: Record<string,string> | null }`. UI показывает текст по ключу `errors.<code>`, `message` — для логов.
- `serde_json::Value` в DTO запрещён (specta не экспортирует рекурсивные типы).
- Даты — строки RFC 3339. u64 экспортируются как `number` (значения < 2^53).
- Токены и пароли никогда не пересекают IPC (кроме пароля в `account_yggdrasil_login`) и никогда не логируются (`Secret`).
- Всё дольше ~100 мс — **задача**: команда принимает `onEvent: Channel<TaskEvent>`, сразу возвращает `TaskId`; результат UI перечитывает обычной командой после `finished`.

### TaskEvent

```ts
type TaskEvent =
  | { type: "started";  id; kind: TaskKind; titleKey: string }
  | { type: "progress"; id; done: number; total: number | null; stage: string; speedBps: number | null }
  | { type: "log";      id; line: string }
  | { type: "finished"; id; error: AppError | null };   // ровно один раз
```

## 3. Tauri-команды

Генерируются в `bindings.ts` как `commands.<camelCase>(...)`, каждая возвращает `{status:"ok",data} | {status:"error",error: AppError}` (кроме `appVersion`). 🕒 — задача (`Channel<TaskEvent>` → `TaskId`).

| Файл (владелец) | Команды |
|---|---|
| `versions.rs` (core) | `versions_list(filter)` |
| `java.rs` (core) | `java_list`, `java_detect`, 🕒`java_install(major)` |
| `instances.rs` (core) | `instance_list`, `instance_get(id)`, 🕒`instance_create(spec)`, 🕒`instance_clone(id,name)`, `instance_update(id,patch)`, `instance_delete(id)`, 🕒`instance_repair(id)`, `instance_game_dir(id)`, 🕒`instance_launch(id,opts)`, `instance_kill(id)`, `instance_state(id)` |
| `system.rs` (core) | `system_info`, `ram_recommend(id)` |
| `loaders.rs` (mods) | `loader_versions(loader, mc)` |
| `content.rs` (mods) | `content_search(query)`, `content_versions(source, projectId, filter)`, `content_resolve(instance, picks)`, 🕒`content_install(instance, plan)`, `content_list(instance)`, `content_set_enabled(instance, fileName, enabled)`, `content_remove(instance, fileName)`, 🕒`content_update_all(instance)`, 🕒`preset_apply(instance, preset)`, 🕒`instance_import(path)`, 🕒`instance_export(id, format, path)` |
| `accounts.rs` (accounts) | `account_list`, `account_offline_add(name)`, `account_yggdrasil_login(apiRoot, username, password)`, `account_yggdrasil_servers`, `account_remove(id)`, `account_set_default(id)` |
| `skins.rs` (accounts) | `skin_library`, `skin_add(pngPath, name, model)`, `skin_remove(id)`, `skin_apply(account, skin)`, `cape_list(account)`, `cape_set(account, capeId)` |
| `vpn.rs` (net) | `vpn_state`, `vpn_set_subscription(urls)`, 🕒`vpn_refresh`, `vpn_set_mode(mode)`, `vpn_nodes`, `vpn_select(node)`, `vpn_test_latency` |
| `rooms.rs` (net) | `room_create(instance, authMode)`, `room_join(code, instance)`, `room_leave`, `room_state`, `room_kick(peerId)`, `chat_send(text)` |
| `voice.rs` (voice) | `voice_devices`, `voice_settings`, `voice_set_settings(settings)`, `voice_start`, `voice_stop`, `voice_set_peer_muted(peerId, muted)`, `voice_set_self_muted(muted)`, `voice_mic_test(enabled)` |
| `settings.rs` (lead) | `settings_get`, `settings_set(settings)` |
| `tasks.rs` (lead) | `tasks_list`, `task_cancel(id)` |
| `app.rs` (release) | `app_version`, `update_check`, 🕒`update_install` |

Правило для файлов команд: только тонкий адаптер (разбор аргументов → вызов сервиса из `AppState`). Бизнес-логика — в крейте роли.

## 4. События

| Имя (Tauri) | `events.<…>` | Payload | Кто эмитит |
|---|---|---|---|
| `tasks://changed` | `tasksChanged` | `TaskSummary[]` | src-tauri (`tasks.rs`) |
| `instance://state` | `instanceState` | `InstanceStatePayload` | core через `GameObserver` |
| `instance://log` | `instanceLog` | `InstanceLogPayload` (пачки по ~100 мс) | core через `GameObserver` |
| `account://changed` | `accountChanged` | `Account[]` | accounts |
| `vpn://state` | `vpnState` | `VpnState` | net |
| `net://blocked` | `netBlocked` | `NetBlockedPayload` | net (lc-http) |
| `room://state` | `roomState` | `RoomState \| null` | net через `RoomObserver` |
| `room://peer` | `roomPeer` | `{ peer, joined }` | net через `RoomObserver` |
| `chat://message` | `chatMessage` | `ChatMessage` | net через `RoomObserver` |
| `voice://speaking` | `voiceSpeaking` | `VoiceSpeakingPayload` (не чаще 10 Гц) | voice через `VoiceObserver` |
| `update://available` | `updateAvailable` | `UpdateInfo` | release |

Сервисы не зависят от Tauri: они получают `*Observer`-трейты, которые реализует `observers.rs`.

## 5. Ключевые трейты (кратко)

```rust
// lc-runtime
trait ProgressSink { fn task_id(); fn emit(TaskEvent); fn cancel_token(); /* progress(), log(), check_cancelled() */ }
trait SettingsStore { async fn get() -> Settings; async fn set(Settings); }
trait LaunchContributor { fn name(); fn contribute(&Instance, &LaunchOpts, &mut LaunchPlan) -> AppResult<()>; }

// lc-http
trait Downloader {
  async fn fetch_many(Vec<DownloadReq>, &dyn ProgressSink) -> AppResult<()>;
  async fn get_bytes(&Url, ResourceKind) -> AppResult<Vec<u8>>;
  fn client(Route) -> reqwest::Client;
  fn mirrors_for(&Url, ResourceKind) -> Vec<Url>;
}

// lc-accounts
trait AuthProvider { async fn launch_auth(Option<AccountId>) -> AppResult<LaunchAuth>; }

// lc-loaders
trait LoaderInstaller { fn kind(); async fn versions(mc); async fn install(&InstallCtx, mc, ver, sink) -> VersionProfileId; }
```

Полные сигнатуры — в файлах `contract.rs`.

## 6. Порядок `LaunchContributor`

`accounts` (authlib-injector javaagent) → `mods` (флаги пресета) → `vpn` (TUN-правила по процессу) → `p2p` (quick play / authlib хоста). Задаётся в `src-tauri`.

## 7. HTTP API

- **hub**: `server/openapi/hub.yaml` (OpenAPI 3.1, проверка `pnpm lint:openapi`). База `https://<domain>/api/v1`, Bearer = Yggdrasil accessToken. Ошибки — формат `AppError`.
- **Yggdrasil**: `server/openapi/yggdrasil.md` (Drasl, спецификация authlib-injector).

## 8. P2P-протокол (`lc-p2p/src/proto.rs`)

- ALPN `lc/1`, одно QUIC-соединение на пару пиров (iroh).
- Первый bi-stream — control: JSON-строки `ControlMsg` (`hello`, `welcome`, `ping/pong`, `chat`, `voice_state`, `kick`, `bye`), тег `t`, snake_case.
- Каждый следующий bi-stream — одно TCP-соединение игры: открывающий пишет строку `StreamOpen` (`{"t":"game"}`), дальше сырые байты.
- Датаграммы — голос: 8-байтный `VoiceHeader` (`ver u8 | speaker u8 | seq u16 | timestamp_ms u32`, big endian) + Opus 20 мс.
- До `MESH_LIMIT = 6` пиров — полная сетка, больше — хост пересылает голос.

## 9. Как меняется контракт

1. Субагент, которому нужно изменение, **останавливает работу над задачей** и пишет запрос в отчёте:

   ```
   CONTRACT REQUEST
   Что: <файл/тип/трейт/команда>
   Зачем: <какую задачу блокирует>
   Предложение: <diff или сигнатура>
   Совместимость: <что сломается у других ролей>
   ```

2. Lead решает, вносит изменение в main отдельным коммитом `contract: …`, перегенерирует `bindings.ts`, обновляет этот файл и стабы.
3. Агенты подтягивают main (`git rebase main` в своём worktree) и продолжают.
