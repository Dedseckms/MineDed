# РљРѕРЅС‚СЂР°РєС‚С‹

Р’Р»Р°РґРµР»РµС† РІСЃРµС… РєРѕРЅС‚СЂР°РєС‚РѕРІ вЂ” **lead**. РЎСѓР±Р°РіРµРЅС‚С‹ РёС… РЅРµ СЂРµРґР°РєС‚РёСЂСѓСЋС‚: РµСЃР»Рё РЅСѓР¶РЅРѕ РёР·РјРµРЅРµРЅРёРµ, Р°РіРµРЅС‚ РѕСЃС‚Р°РЅР°РІР»РёРІР°РµС‚СЃСЏ Рё РїРёС€РµС‚ Р·Р°РїСЂРѕСЃ (С„РѕСЂРјР°С‚ РІ РєРѕРЅС†Рµ С„Р°Р№Р»Р°). РћР±РѕСЃРЅРѕРІР°РЅРёРµ РїРѕРґС…РѕРґР° вЂ” ADR-0004.

## 1. Р“РґРµ С‡С‚Рѕ Р»РµР¶РёС‚

| РљРѕРЅС‚СЂР°РєС‚ | Р¤Р°Р№Р»(С‹) | РџРѕС‚СЂРµР±РёС‚РµР»Рё |
|---|---|---|
| DTO, ID, `AppError`, `TaskEvent`, payload СЃРѕР±С‹С‚РёР№ | `crates/lc-types/src/*.rs` | РІСЃРµ РєСЂРµР№С‚С‹, С„СЂРѕРЅС‚ (С‡РµСЂРµР· bindings) |
| РџСЂРѕРіСЂРµСЃСЃ, РѕС‚РјРµРЅР°, РїСѓС‚Рё, РЅР°СЃС‚СЂРѕР№РєРё, РїР»Р°РЅ Р·Р°РїСѓСЃРєР° | `crates/lc-runtime/src/*.rs` | РІСЃРµ backend-РєСЂРµР№С‚С‹ |
| `Downloader`, `Route`, `DownloadReq` | `crates/lc-http/src/contract.rs` | core, mods, accounts, vpn |
| `VersionService`, `JavaManager`, `InstanceService`, `GameLauncher`, `GameObserver`, `SystemProbe` | `crates/lc-core/src/contract.rs` | src-tauri |
| `LoaderInstaller`, `InstallCtx`, `LoaderRegistry` | `crates/lc-loaders/src/contract.rs` | core, src-tauri |
| `ContentSource`, `ContentService`, `PackIo`, `PresetService` | `crates/lc-content/src/contract.rs` | src-tauri |
| `AuthProvider`, `LaunchAuth`, `AccountService`, `TokenStore`, `SkinService`, `Secret` | `crates/lc-accounts/src/contract.rs` | core, src-tauri |
| `VpnController` | `crates/lc-vpn/src/contract.rs` | src-tauri, lc-http (proxy URL) |
| `RoomService`, `DatagramTransport`, `RoomObserver` | `crates/lc-p2p/src/contract.rs` | src-tauri, voice |
| P2P wire protocol (`ALPN`, `ControlMsg`, `StreamOpen`, `VoiceHeader`) | `crates/lc-p2p/src/proto.rs` | net, voice |
| `VoiceEngine`, `VoiceObserver` | `crates/lc-voice/src/contract.rs` | src-tauri |
| Р РµРµСЃС‚СЂ РєРѕРјР°РЅРґ Рё СЃРѕР±С‹С‚РёР№ | `apps/desktop/src-tauri/src/bindings.rs`, `events.rs` | С„СЂРѕРЅС‚ |
| РЎРµСЂРІРёСЃРЅС‹Р№ РєРѕРЅС‚РµР№РЅРµСЂ, Р·Р°РґР°С‡Рё, РЅР°Р±Р»СЋРґР°С‚РµР»Рё | `apps/desktop/src-tauri/src/{state,tasks,observers}.rs` | вЂ” |
| TS-С‚РёРїС‹ Рё РѕР±С‘СЂС‚РєРё РєРѕРјР°РЅРґ (**Р°РІС‚РѕРіРµРЅ**) | `apps/desktop/src/bindings.ts` | ui |
| HTTP API hub | `server/openapi/hub.yaml` | hub, lc-p2p, lc-http, lc-accounts |
| Yggdrasil (Drasl) | `server/openapi/yggdrasil.md` | lc-accounts, server-ops |

РљР°Р¶РґС‹Р№ С‚СЂРµР№С‚ РёРјРµРµС‚ `Stub*`-СЂРµР°Р»РёР·Р°С†РёСЋ РІ `src/stub.rs` С‚РѕРіРѕ Р¶Рµ РєСЂРµР№С‚Р°; СЃС‚Р°Р± РІРѕР·РІСЂР°С‰Р°РµС‚ `ErrorCode::NotImplemented` (РёР»Рё РїСѓСЃС‚РѕР№ СЃРїРёСЃРѕРє/В«РІС‹РєР»СЋС‡РµРЅРѕВ»). Р—Р°РјРµРЅР° СЃС‚Р°Р±Р° СЂРµР°Р»СЊРЅРѕР№ СЂРµР°Р»РёР·Р°С†РёРµР№ РґРµР»Р°РµС‚СЃСЏ **С‚РѕР»СЊРєРѕ** РІ `apps/desktop/src-tauri/src/state.rs` (lead).

## 2. РћР±С‰РёРµ РїСЂР°РІРёР»Р°

- РќР° РїСЂРѕРІРѕРґРµ РІСЃС‘ РІ **camelCase** (`#[serde(rename_all = "camelCase")]`), enum'С‹ вЂ” СЃС‚СЂРѕРєРё РІ camelCase.
- РћС€РёР±РєР° С‡РµСЂРµР· IPC/РјРµР¶РґСѓ РєСЂРµР№С‚Р°РјРё вЂ” С‚РѕР»СЊРєРѕ `AppError { code: ErrorCode, message, details: Record<string,string> | null }`. UI РїРѕРєР°Р·С‹РІР°РµС‚ С‚РµРєСЃС‚ РїРѕ РєР»СЋС‡Сѓ `errors.<code>`, `message` вЂ” РґР»СЏ Р»РѕРіРѕРІ.
- `serde_json::Value` РІ DTO Р·Р°РїСЂРµС‰С‘РЅ (specta РЅРµ СЌРєСЃРїРѕСЂС‚РёСЂСѓРµС‚ СЂРµРєСѓСЂСЃРёРІРЅС‹Рµ С‚РёРїС‹).
- Р”Р°С‚С‹ вЂ” СЃС‚СЂРѕРєРё RFC 3339. u64 СЌРєСЃРїРѕСЂС‚РёСЂСѓСЋС‚СЃСЏ РєР°Рє `number` (Р·РЅР°С‡РµРЅРёСЏ < 2^53).
- РўРѕРєРµРЅС‹ Рё РїР°СЂРѕР»Рё РЅРёРєРѕРіРґР° РЅРµ РїРµСЂРµСЃРµРєР°СЋС‚ IPC (РёСЃРєР»СЋС‡РµРЅРёСЏ: РїР°СЂРѕР»СЊ РІ `account_yggdrasil_login` Рё РїР°СЂРѕР»СЊ СЂСѓС‡РЅРѕРіРѕ РїСЂРѕРєСЃРё РІ `Settings.proxy`) Рё РЅРёРєРѕРіРґР° РЅРµ Р»РѕРіРёСЂСѓСЋС‚СЃСЏ (`Secret`, СЂСѓС‡РЅРѕР№ `Debug` Сѓ `ProxyCfg`).
- Р—РµСЂРєР°Р»Р° (BMCLAPI Рё С‚.Рї.) РЅРµРґРѕРІРµСЂРµРЅРЅС‹Рµ: РґР»СЏ Р·Р°РїСЂРѕСЃРѕРІ **Р±РµР·** `sha1` СЃС‚РѕСЂРѕРЅРЅРёРµ Р·РµСЂРєР°Р»Р° РёСЃРїРѕР»СЊР·СѓСЋС‚СЃСЏ С‚РѕР»СЊРєРѕ РїРѕСЃР»Рµ РѕСЂРёРіРёРЅР°Р»Р°, РЅРµР·Р°РІРёСЃРёРјРѕ РѕС‚ `prefer_mirrors`; core РїРµСЂРµРґР°С‘С‚ `sha1` РІРµР·РґРµ, РіРґРµ РѕРЅ РёР·РІРµСЃС‚РµРЅ.
- Р’СЃС‘ РґРѕР»СЊС€Рµ ~100 РјСЃ вЂ” **Р·Р°РґР°С‡Р°**: РєРѕРјР°РЅРґР° РїСЂРёРЅРёРјР°РµС‚ `onEvent: Channel<TaskEvent>`, СЃСЂР°Р·Сѓ РІРѕР·РІСЂР°С‰Р°РµС‚ `TaskId`; СЂРµР·СѓР»СЊС‚Р°С‚ UI РїРµСЂРµС‡РёС‚С‹РІР°РµС‚ РѕР±С‹С‡РЅРѕР№ РєРѕРјР°РЅРґРѕР№ РїРѕСЃР»Рµ `finished`.

### TaskEvent

```ts
type TaskEvent =
  | { type: "started";  id; kind: TaskKind; titleKey: string }
  | { type: "progress"; id; done: number; total: number | null; stage: string; speedBps: number | null }
  | { type: "log";      id; line: string }
  | { type: "finished"; id; error: AppError | null };   // СЂРѕРІРЅРѕ РѕРґРёРЅ СЂР°Р·
```

## 3. Tauri-РєРѕРјР°РЅРґС‹

Р“РµРЅРµСЂРёСЂСѓСЋС‚СЃСЏ РІ `bindings.ts` РєР°Рє `commands.<camelCase>(...)`, РєР°Р¶РґР°СЏ РІРѕР·РІСЂР°С‰Р°РµС‚ `{status:"ok",data} | {status:"error",error: AppError}` (РєСЂРѕРјРµ `appVersion`). рџ•’ вЂ” Р·Р°РґР°С‡Р° (`Channel<TaskEvent>` в†’ `TaskId`).

| Р¤Р°Р№Р» (РІР»Р°РґРµР»РµС†) | РљРѕРјР°РЅРґС‹ |
|---|---|
| `versions.rs` (core) | `versions_list(filter)` |
| `java.rs` (core) | `java_list`, `java_detect`, рџ•’`java_install(major)` |
| `instances.rs` (core) | `instance_list`, `instance_get(id)`, рџ•’`instance_create(spec)`, рџ•’`instance_clone(id,name)`, `instance_update(id,patch)`, `instance_delete(id)`, рџ•’`instance_repair(id)`, `instance_game_dir(id)`, рџ•’`instance_launch(id,opts)`, `instance_kill(id)`, `instance_state(id)` |
| `system.rs` (core) | `system_info`, `ram_recommend(id)` |
| `loaders.rs` (mods) | `loader_versions(loader, mc)` |
| `content.rs` (mods) | `content_search(query)`, `content_versions(source, projectId, filter)`, `content_resolve(instance, picks)`, рџ•’`content_install(instance, plan)`, `content_list(instance)`, `content_set_enabled(instance, fileName, enabled)`, `content_remove(instance, fileName)`, рџ•’`content_update_all(instance)`, рџ•’`preset_apply(instance, preset)`, рџ•’`instance_import(path)`, рџ•’`instance_export(id, format, path)` |
| `accounts.rs` (accounts) | `account_list`, `account_offline_add(name)`, `account_yggdrasil_login(apiRoot, username, password)`, `account_yggdrasil_register(apiRoot, invite, username, password)`, `account_yggdrasil_servers`, `account_remove(id)`, `account_set_default(id)` |
| `skins.rs` (accounts) | `skin_library`, `skin_add(pngPath, name, model)`, `skin_remove(id)`, `skin_apply(account, skin)`, `cape_list(account)`, `cape_set(account, capeId)` |
| `vpn.rs` (net) | `vpn_state`, `vpn_set_subscription(urls)`, рџ•’`vpn_refresh`, `vpn_set_mode(mode)`, `vpn_nodes`, `vpn_select(node)`, `vpn_test_latency` |
| `rooms.rs` (net) | `room_create(instance, authMode)`, `room_join(code, instance)`, `room_leave`, `room_state`, `room_kick(peerId)`, `chat_send(text)` |
| `voice.rs` (voice) | `voice_devices`, `voice_settings`, `voice_set_settings(settings)`, `voice_start`, `voice_stop`, `voice_set_peer_muted(peerId, muted)`, `voice_set_self_muted(muted)`, `voice_mic_test(enabled)` |
| `settings.rs` (lead) | `settings_get`, `settings_set(settings)` |
| `tasks.rs` (lead) | `tasks_list`, `task_cancel(id)` |
| `app.rs` (release) | `app_version`, `update_check`, рџ•’`update_install` |

РџСЂР°РІРёР»Рѕ РґР»СЏ С„Р°Р№Р»РѕРІ РєРѕРјР°РЅРґ: С‚РѕР»СЊРєРѕ С‚РѕРЅРєРёР№ Р°РґР°РїС‚РµСЂ (СЂР°Р·Р±РѕСЂ Р°СЂРіСѓРјРµРЅС‚РѕРІ в†’ РІС‹Р·РѕРІ СЃРµСЂРІРёСЃР° РёР· `AppState`). Р‘РёР·РЅРµСЃ-Р»РѕРіРёРєР° вЂ” РІ РєСЂРµР№С‚Рµ СЂРѕР»Рё.

## 4. РЎРѕР±С‹С‚РёСЏ

| РРјСЏ (Tauri) | `events.<вЂ¦>` | Payload | РљС‚Рѕ СЌРјРёС‚РёС‚ |
|---|---|---|---|
| `tasks://changed` | `tasksChanged` | `TaskSummary[]` | src-tauri (`tasks.rs`) |
| `instance://state` | `instanceState` | `InstanceStatePayload` | core С‡РµСЂРµР· `GameObserver` |
| `instance://log` | `instanceLog` | `InstanceLogPayload` (РїР°С‡РєРё РїРѕ ~100 РјСЃ) | core С‡РµСЂРµР· `GameObserver` |
| `account://changed` | `accountChanged` | `Account[]` | accounts |
| `vpn://state` | `vpnState` | `VpnState` | net |
| `net://blocked` | `netBlocked` | `NetBlockedPayload` | net (lc-http) |
| `room://state` | `roomState` | `RoomState \| null` | net С‡РµСЂРµР· `RoomObserver` |
| `room://peer` | `roomPeer` | `{ peer, joined }` | net С‡РµСЂРµР· `RoomObserver` |
| `chat://message` | `chatMessage` | `ChatMessage` | net С‡РµСЂРµР· `RoomObserver` |
| `voice://speaking` | `voiceSpeaking` | `VoiceSpeakingPayload` (РЅРµ С‡Р°С‰Рµ 10 Р“С†) | voice С‡РµСЂРµР· `VoiceObserver` |
| `update://available` | `updateAvailable` | `UpdateInfo` | release |

РЎРµСЂРІРёСЃС‹ РЅРµ Р·Р°РІРёСЃСЏС‚ РѕС‚ Tauri: РѕРЅРё РїРѕР»СѓС‡Р°СЋС‚ `*Observer`-С‚СЂРµР№С‚С‹, РєРѕС‚РѕСЂС‹Рµ СЂРµР°Р»РёР·СѓРµС‚ `observers.rs`.

## 5. РљР»СЋС‡РµРІС‹Рµ С‚СЂРµР№С‚С‹ (РєСЂР°С‚РєРѕ)

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

РџРѕР»РЅС‹Рµ СЃРёРіРЅР°С‚СѓСЂС‹ вЂ” РІ С„Р°Р№Р»Р°С… `contract.rs`.

## 6. РџРѕСЂСЏРґРѕРє `LaunchContributor`

`accounts` (authlib-injector javaagent) в†’ `mods` (С„Р»Р°РіРё РїСЂРµСЃРµС‚Р°) в†’ `vpn` (TUN-РїСЂР°РІРёР»Р° РїРѕ РїСЂРѕС†РµСЃСЃСѓ) в†’ `p2p` (quick play / authlib С…РѕСЃС‚Р°). Р—Р°РґР°С‘С‚СЃСЏ РІ `src-tauri`.

## 7. HTTP API

- **hub**: `server/openapi/hub.yaml` (OpenAPI 3.1, РїСЂРѕРІРµСЂРєР° `pnpm lint:openapi`). Р‘Р°Р·Р° `https://<domain>/api/v1`, Bearer = Yggdrasil accessToken. РћС€РёР±РєРё вЂ” С„РѕСЂРјР°С‚ `AppError`.
- **Yggdrasil**: `server/openapi/yggdrasil.md` (Drasl, СЃРїРµС†РёС„РёРєР°С†РёСЏ authlib-injector).

## 8. P2P-РїСЂРѕС‚РѕРєРѕР» (`lc-p2p/src/proto.rs`)

- ALPN `lc/1`, РѕРґРЅРѕ QUIC-СЃРѕРµРґРёРЅРµРЅРёРµ РЅР° РїР°СЂСѓ РїРёСЂРѕРІ (iroh).
- РџРµСЂРІС‹Р№ bi-stream вЂ” control: JSON-СЃС‚СЂРѕРєРё `ControlMsg` (`hello`, `welcome`, `ping/pong`, `chat`, `voice_state`, `kick`, `bye`), С‚РµРі `t`, snake_case.
- РљР°Р¶РґС‹Р№ СЃР»РµРґСѓСЋС‰РёР№ bi-stream вЂ” РѕРґРЅРѕ TCP-СЃРѕРµРґРёРЅРµРЅРёРµ РёРіСЂС‹: РѕС‚РєСЂС‹РІР°СЋС‰РёР№ РїРёС€РµС‚ СЃС‚СЂРѕРєСѓ `StreamOpen` (`{"t":"game"}`), РґР°Р»СЊС€Рµ СЃС‹СЂС‹Рµ Р±Р°Р№С‚С‹.
- Р”Р°С‚Р°РіСЂР°РјРјС‹ вЂ” РіРѕР»РѕСЃ: 8-Р±Р°Р№С‚РЅС‹Р№ `VoiceHeader` (`ver u8 | speaker u8 | seq u16 | timestamp_ms u32`, big endian) + Opus 20 РјСЃ.
- Р”Рѕ `MESH_LIMIT = 6` РїРёСЂРѕРІ вЂ” РїРѕР»РЅР°СЏ СЃРµС‚РєР°, Р±РѕР»СЊС€Рµ вЂ” С…РѕСЃС‚ РїРµСЂРµСЃС‹Р»Р°РµС‚ РіРѕР»РѕСЃ.

## 9. РљР°Рє РјРµРЅСЏРµС‚СЃСЏ РєРѕРЅС‚СЂР°РєС‚

1. РЎСѓР±Р°РіРµРЅС‚, РєРѕС‚РѕСЂРѕРјСѓ РЅСѓР¶РЅРѕ РёР·РјРµРЅРµРЅРёРµ, **РѕСЃС‚Р°РЅР°РІР»РёРІР°РµС‚ СЂР°Р±РѕС‚Сѓ РЅР°Рґ Р·Р°РґР°С‡РµР№** Рё РїРёС€РµС‚ Р·Р°РїСЂРѕСЃ РІ РѕС‚С‡С‘С‚Рµ:

   ```
   CONTRACT REQUEST
   Р§С‚Рѕ: <С„Р°Р№Р»/С‚РёРї/С‚СЂРµР№С‚/РєРѕРјР°РЅРґР°>
   Р—Р°С‡РµРј: <РєР°РєСѓСЋ Р·Р°РґР°С‡Сѓ Р±Р»РѕРєРёСЂСѓРµС‚>
   РџСЂРµРґР»РѕР¶РµРЅРёРµ: <diff РёР»Рё СЃРёРіРЅР°С‚СѓСЂР°>
   РЎРѕРІРјРµСЃС‚РёРјРѕСЃС‚СЊ: <С‡С‚Рѕ СЃР»РѕРјР°РµС‚СЃСЏ Сѓ РґСЂСѓРіРёС… СЂРѕР»РµР№>
   ```

2. Lead СЂРµС€Р°РµС‚, РІРЅРѕСЃРёС‚ РёР·РјРµРЅРµРЅРёРµ РІ main РѕС‚РґРµР»СЊРЅС‹Рј РєРѕРјРјРёС‚РѕРј `contract: вЂ¦`, РїРµСЂРµРіРµРЅРµСЂРёСЂСѓРµС‚ `bindings.ts`, РѕР±РЅРѕРІР»СЏРµС‚ СЌС‚РѕС‚ С„Р°Р№Р» Рё СЃС‚Р°Р±С‹.
3. РђРіРµРЅС‚С‹ РїРѕРґС‚СЏРіРёРІР°СЋС‚ main (`git rebase main` РІ СЃРІРѕС‘Рј worktree) Рё РїСЂРѕРґРѕР»Р¶Р°СЋС‚.
