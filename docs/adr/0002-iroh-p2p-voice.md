# ADR-0002: iroh (QUIC) для P2P-игры, чата и голоса вместо WebRTC

Статус: принято (2026-10-20)

**Контекст.** Нужны P2P-сессии по коду комнаты с NAT traversal и relay-fallback, текстовый и голосовой чат. WebRTC в WebKitGTK на Linux ненадёжен; webrtc-rs тяжёл и плохо подходит для туннеля TCP.

**Решение.** iroh (QUIC + hole punching + self-hosted `iroh-relay`):
- игра: каждое TCP-соединение Minecraft — отдельный QUIC bi-stream к LAN-порту хоста;
- контроль и чат: первый bi-stream, JSON-строки (`lc-p2p::proto::ControlMsg`);
- голос: Opus 20 мс в QUIC-датаграммах (`VoiceHeader`), cpal для аудио;
- hub выдаёт коды комнат и токены; WebSocket hub — fallback для чата.

**Последствия.**
- Один сетевой стек, relay из коробки.
- Нет встроенных AEC/NS/AGC WebRTC → `webrtc-audio-processing` или `nnnoiseless`; push-to-talk снижает проблему эха.
- Версию iroh фиксируем точно (API менялся между версиями).
