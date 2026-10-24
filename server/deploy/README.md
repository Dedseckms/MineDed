# Deploy (owner: server-ops)

Server access is SSH-only, under the rules in the service DEPLOY.md files.

## Layout on the server

```
/opt/launcher/            git checkout of server/ (read-only for services)
/opt/launcher/.env        secrets (chmod 600, never committed)
volumes                   caddy_data, drasl_state, mirror_cache
```

## Services

| Service | Public | Purpose |
|---|---|---|
| caddy | 443/tcp, 443/udp, 80 | TLS, routing: `/` → drasl, `/api/v1` → hub, `/mirror/` → mirror-cache, `relay.` → iroh-relay |
| drasl | — | Yggdrasil auth + textures (`server/openapi/yggdrasil.md`) |
| hub | — | Rooms, mirrors list, skin library (`server/openapi/hub.yaml`) |
| iroh-relay | 3478/udp | P2P relay fallback + address discovery |
| mirror-cache | — | GET-only nginx cache for Mojang/Modrinth files (hash-verified by the client) |

## Open questions (wave 0)

- Public IP / port forwarding for 443 and 3478/udp; otherwise a VPS for the relay.
- Domain name and DNS records.
- Pinned image tags.
