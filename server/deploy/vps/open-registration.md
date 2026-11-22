# Open password registration — prepared 2026-11-22

Status: applied with owner approval on 2026-11-22 at 07:28 UTC. See the actual
backup, verification results and rollback in [DEPLOY.md](DEPLOY.md).

## Read-only baseline

- `/opt/skinserver/compose.yaml`, project `mineup-skin`, service `drasl`, container
  `mineup-skin-drasl`, image `mineup/drasl:4.0.1-hc-theme2`.
- `/opt/skinserver/config` is mounted read-only at `/etc/drasl`; persistent state is
  volume `mineup-skin_drasl_state`.
- Live `RegistrationUsernamePassword.CreateNewPlayer`: `Allow=true`,
  `RequireInvite=true`, `AllowChoosingUUID=false`.
- Password minimum 10, one player by default, extra player creation disabled;
  rate limiter enabled at 2 requests/s, burst 20.
- Compose validation passed; Drasl healthy, metadata version 4.0.1; hub
  `/api/v1/health` returned `{"status":"ok","version":"1.6.0"}`.
- SHA-256 of the metadata signing public key:
  `f9d570264fb4084abbfe84018fae596930fdc9a2e3f839f8602c13755ace5af5`.
- Official setting semantics: [Drasl v4.0.1 configuration](https://github.com/unmojang/drasl/blob/v4.0.1/doc/configuration.md#L67-L71).
- The candidate transformation was validated in memory against the live TOML:
  parsed configuration differs only in `RequireInvite=false`; the file remained
  unchanged. `pnpm lint:openapi` and `git diff --check` passed locally.

## Apply after approval

Run from `/opt/skinserver` using the existing operator access. Change the live
file in place; do not replace it with the repository template.

```bash
set -eu
cd /opt/skinserver
stamp=$(date -u +%Y%m%dT%H%M%SZ)
backup="backups/config-open-registration-$stamp.toml"
cp -p config/config.toml "$backup"
python3 - <<'PY'
from pathlib import Path
import copy, re, tomllib
p = Path('config/config.toml')
raw = p.read_bytes()
before = tomllib.loads(raw.decode())
policy = before['RegistrationUsernamePassword']['CreateNewPlayer']
assert policy['Allow'] is True and policy['RequireInvite'] is True
updated, count = re.subn(
    rb'(?m)^(\[RegistrationUsernamePassword\.CreateNewPlayer\][^\r\n]*\r?\n)'
    rb'((?:(?!^\[).)*?)(^RequireInvite\s*=\s*)true(\s*(?:#[^\r\n]*)?$)',
    rb'\g<1>\g<2>\g<3>false\g<4>', raw, flags=re.DOTALL)
assert count == 1
expected = copy.deepcopy(before)
expected['RegistrationUsernamePassword']['CreateNewPlayer']['RequireInvite'] = False
assert tomllib.loads(updated.decode()) == expected
with p.open('r+b') as f:
    f.write(updated)
    f.truncate()
print('Only RequireInvite changed; TOML parsed successfully')
PY
docker compose -p mineup-skin config --quiet
docker compose -p mineup-skin restart --no-deps drasl
```

Stop on any failed command. Record the actual backup filename. Wait for the
existing container health check to become healthy. Verify local and public
Drasl metadata, unchanged signing public key, `/web/registration`, and hub health.
Confirm the registration form no longer requires an invite and, if an approved
test account is available, verify password registration and login through the
launcher. Do not create test users without a cleanup decision.

Append the actual outcome, backup path and rollback command to
`/opt/skinserver/DEPLOY.md` and the repository deployment log. No state volume,
image, account data, authentication settings or other services need changing.

## Rollback

Restore the recorded configuration backup and restart only Drasl:

```bash
set -eu
cd /opt/skinserver
cp -p backups/config-open-registration-<recorded-stamp>.toml config/config.toml
docker compose -p mineup-skin config --quiet
docker compose -p mineup-skin restart --no-deps drasl
```

Repeat the same health and signing-key checks. Existing accounts remain present;
restoring invite requirements does not delete accounts created while registration
was open.
