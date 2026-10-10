# The XCHANGE host (do-sgp1): what is installed, and what is still to build

> written-by: claude/2b9ffb · measured on the host on 2026-10-09 (~04:00Z) · repo `main` at the time of the PR

This page is for **ticket issuers and the lane controller**: what runs on the XCHANGE host today, how each piece
was installed, how to update or remove it, and which work is still open (section 6). Nothing here is a
decision; decisions are quoted with their source.

| | |
|---|---|
| Host | DigitalOcean droplet `do-sgp1`, 152.42.247.83 · Ubuntu 24.04 · 2 vCPU · 3.8 GB RAM (≈2.2 GB available) · 116 GB disk (21 % used) |
| Shared with | other `*.spacing.cloud` apps (chainslip, xc-desk-v2 = v2, xc-exchange, financedash). **Do not touch them.** |
| Reach | `ssh do-sgp1` (root) · `ssh do-sgp1-yuth` (user `yuth`, sudo) |
| Build | Rust binaries are built on Dell and copied here (owner's decision 2026-10-06); nothing is compiled on this host |

## 1. Front door: Cloudflare and nginx

- Every public name is a **proxied** Cloudflare record; the zone runs **Full (Strict)**, so each vhost needs its own
  certificate on `:443` (Let's Encrypt, webroot `/var/www/acme`, renewed by `certbot.timer`). A vhost without `:443`
  answers Cloudflare **526**.
- **The origin accepts web traffic from Cloudflare only** (owner 2026-10-09): firewall chain `CF-ONLY`
  (iptables + ip6tables) on tcp 80/443 allows loopback and Cloudflare's ranges, nothing else. Persisted in
  `/etc/iptables/rules.v{4,6}`. Undo steps: `/root/cf-only-firewall.README`.
- **Weekly range check:** `/etc/cron.d/cf-ranges-sync` (Mondays 03:17 Bangkok) runs
  `/usr/local/sbin/cf-ranges-sync.sh`, which rebuilds `CF-ONLY` and `/etc/nginx/conf.d/cloudflare-realip.conf`
  when Cloudflare publishes new ranges. Last result: `/root/cf-ranges.status`.

| Public name | vhost (`/etc/nginx/sites-available/`) | Goes to |
|---|---|---|
| `xc-v3.spacing.cloud` | `xc-v3.spacing.cloud.conf` | XCHANGE v3 server, `127.0.0.1:8094` (staff, admin, desk) |
| `xc-v3-customer.spacing.cloud` | `xc-v3-customer.spacing.cloud.conf` | Flutter web kiosk from `/var/www/xc-v3-customer/web`; only `POST /api/kiosk` reaches `:8094` |
| `xc-id.spacing.cloud` | `xc-id.spacing.cloud.conf` | XCHANGE Identity, `127.0.0.1:8095` (DNS + Let's Encrypt certificate + `:443` since 2026-10-09) |

## 2. XCHANGE v3 server (`xc-v3.service`)

Installed and updated with `deploy/server/` (`build.md`, `install.sh`, `update.sh`, `smoke.sh`). Binary
`/opt/xc-v3/xchange`, env `/etc/xc-v3/env`, database `xc_v3` (local PostgreSQL 16), uploads `/var/lib/xc-v3/uploads`,
daily encrypted backup `xc-v3-backup.timer`. `update.sh` backs up first and keeps `.prev` copies.
Test data only (D6); the channel reports `staging`.

**Identity settings come from a drop-in the installer does not make.** The unit `deploy/server/xc-v3.service`
loads only `/etc/xc-v3/env` (`EnvironmentFile=`, rendered by `deploy/server/common.sh` `render_unit`), and nothing
in `deploy/server/` mentions `/etc/xc-v3/identity-client.env` (section 4). On this host the owner added
`/etc/systemd/system/xc-v3.service.d/identity.conf` with `EnvironmentFile=/etc/xc-v3/identity-client.env`,
written by the lane controller's `set-identity-api-key.sh` (the owner ran it on 2026-10-10 around 19:35Z). Without that drop-in the server runs with Identity off:
the sign-in routes (`/api/auth/login`, `/api/auth/otp/*`) answer `503` (`api/tests/identity_signin_test.rs`), and nobody can sign in.
- **Fresh install or rebuilt host:** write `/etc/systemd/system/xc-v3.service.d/identity.conf` with exactly
  this content (systemd ignores a drop-in without the `[Service]` header):

  ```ini
  [Service]
  EnvironmentFile=/etc/xc-v3/identity-client.env
  ```

  Then run `systemctl daemon-reload` and `systemctl restart xc-v3`. Check that `systemctl cat xc-v3` lists both
  `EnvironmentFile=` lines.
- **Remove:** delete the drop-in, `daemon-reload`, restart. Sign-in stops working.

## 3. Kiosk web (Flutter)

`app/` built with `flutter build web --release`, copied to `/var/www/xc-v3-customer/web` (old copy kept as
`web.prev`, build sha in `/var/www/xc-v3-customer/BUILD`). No service of its own; nginx serves the files.

## 4. XCHANGE Identity (`xc-identity.service`) — new 2026-10-09

Owner's order: *"Identity จะต้อง deploy ที่โฮสต์ xchange ไม่ยุ่งกับของเดิม"* — a **separate instance**, not the
Identity on Dell (its database, keys, Redis and clients are all its own; nothing is shared with Dell).

| Item | Value |
|---|---|
| Program | `identity-api` from `108-Plaza/108-platform-services` (`crates/identity-api`), built on Dell at `681bea2`, copied to `/opt/xc-identity/identity-api` (build sha in `/opt/xc-identity/BUILD`) |
| Runs as | system user `xc-identity`, unit `/etc/systemd/system/xc-identity.service` (`ProtectSystem=strict`, restart on failure) |
| Listens | `127.0.0.1:8095` only; public through the `xc-id` vhost |
| Env | `/etc/xc-identity/env` (root:xc-identity 640). All secrets were **generated on this host** and are not stored anywhere else: `AUTH_JWT_SIGNING_KEY` (new Ed25519, kid `env-d7aae4675bd2`), `AUTH_API_KEY`, `INTERNAL_SERVICE_TOKEN`, `SESSION_TOKEN_HASH_SECRET`, `AUTH_OTP_PEPPER`, `AUTH_TOTP_ENC_KEY`, `AUTH_TENANT_ID` |
| Settings | `AUTH_FAIL_CLOSED=1` · `AUTH_ISSUER=auth-service` · `AUTH_AUDIENCE=xchange` · `AUTH_OTP_DELIVERY_MODE=noop` (no SMS/email yet) · Google sign-in off · CORS `https://xc-v3.spacing.cloud`, `https://xc-v3-customer.spacing.cloud` · passkey RP `xc-id.spacing.cloud` |
| Database | `xc_identity` (role `xc_identity`) on the local PostgreSQL; schemas migrate at boot |
| Redis | `redis://127.0.0.1:6379/5` (db 5 is this service's alone) |
| Memory | ≈18 MB resident |
| Client for xchange | `name=xchange`, `appType=web`, `audience=xchange`, clientId `e06ca64f-2d57-44f5-96be-950ead06b76c`. Credentials for the xchange server: `/etc/xc-v3/identity-client.env` (root:xc-v3 640: `IDENTITY_BASE_URL`, `IDENTITY_PUBLIC_URL`, `IDENTITY_CLIENT_ID`, `IDENTITY_CLIENT_SECRET`, `IDENTITY_AUDIENCE`, `IDENTITY_ISSUER`) |

**Proven on 2026-10-09** with a throw-away account (deleted afterwards; the database holds 0 identities):
a login sent with the xchange client returns `200`, a token with `iss=auth-service`, `aud=xchange`, 15-minute life,
whose signature verifies against `http://127.0.0.1:8095/.well-known/jwks.json`; a wrong client secret returns
`401 invalid_client_credentials`. Registration requires a phone number (`phone_required`).

**Update:** build `identity-api` on Dell at the new sha, copy over `/opt/xc-identity/identity-api`, write the sha to
`BUILD`, `systemctl restart xc-identity`, check `/health` and the JWKS. Keys survive restarts (they are in the env).
**Remove:** `systemctl disable --now xc-identity`, delete the unit, `/opt/xc-identity`, `/etc/xc-identity`, the vhost,
`DROP DATABASE xc_identity` and `redis-cli -n 5 FLUSHDB` — only on the owner's order.

The two `xchange` clients first registered by mistake in the **Dell** Identity (prod `7cd71b33…`, staging
`f5520181…`) were **disabled** the same day; nothing on Dell uses them.

## 5. Recognize (OCR) — not on this host (yet)

The owner first chose to run Recognize as a service on this host, then decided on 2026-10-09:
*"ถ้าไม่ไหว ให้ ใช้ dell ก่อน"* → *"เอา ที่ dell ก่อนละกัน"*. **The passport reader runs on Dell first.**
The v3 server on this host calls it over HTTPS through a public name (Dell has no route to the droplet).
`108-Plaza/Recognize-Platform` today is an on-device library (Rust + ONNX, Flutter plugin, WASM) with a CLI and
**no HTTP server**; the service is designed in `Recognize-Platform#150`.

## 6. Work that ticket issuers need to cut

| # | Work | Where the ticket belongs | Notes |
|---|---|---|---|
| I-1 | ~~DNS record `xc-id.spacing.cloud`, certbot + `:443`~~ | — | **Done 2026-10-09**: `https://xc-id.spacing.cloud/health` 200, JWKS 200, `/admin/*` 401 without the internal token |
| I-2 | The xchange server verifies Identity tokens: `aud=xchange`, `iss=auth-service`, keys from `xc-id`'s JWKS; reads `/etc/xc-v3/identity-client.env` | `108-Plaza/xchange` — design #550 (PR 589 §5) and #554 | **Everyone signs in through xc-identity: staff, the owner and customers.** The owner answered on 2026-10-09 "Follow the Blueprint: Identity for everyone" (PR 589 §0 B2), which reverses the 10-06 D4 order that xc-auth keeps staff accounts. An xchange user is bound to an Identity account by a one-time invite code (PR 589 §5.3) |
| I-3 | Customer sign-in flow in the app (register by phone, OTP delivery, optional Google) | `108-Plaza/xchange` — #558 (customer app), #550 (staff) | OTP is `noop` today: an SMS/e-mail sender must be chosen (owner). Identity registration **requires a phone** (`phone_required`), so staff register with their own phones too |
| I-4 | `/health` reports `build: unknown` | `108-Plaza/108-platform-services` | The fleet versioning rule wants `build`/`builtAt`/`channel`; set them at build time |
| R-1 | Design a Recognize HTTP service that fits 2 vCPU / ~2 GB: which documents (passport MRZ, Thai ID), which engine, concurrency limit, request size, no image kept after the answer | **already open: `108-Plaza/Recognize-Platform#150`** — host: **Dell first** (owner) (claude/b060e9, carries v2's MRZ behaviour contract) — do not open a second ticket | Blueprint §16 rules apply: identity documents are never logged or exported |
| R-2 | The xchange desk/kiosk calls that service instead of typing passport fields | `108-Plaza/xchange` — #557 (verification) | After R-1 merges |
| P-1 | Passport data at rest: encrypted value + lookup hash (#554), documents in a private store with `retention_until` (#557) | `108-Plaza/xchange` | Retention period is a §12 value the owner must set (legal) |
