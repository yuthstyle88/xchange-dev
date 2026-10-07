# Deployment files

Everything a host needs to run XCHANGE is described here. **Nothing in this
directory is applied by merging it, by CI, or by any agent: every step on a host
— installing, updating, creating an account, changing nginx — is done only on the
owner's order, and nginx on a host is its owner's to change.** `APP_ENVIRONMENT`
and lane names say which settings file is read, not that a real shop is using it.

| File | What it is |
|---|---|
| `server/xc-v3.service` | systemd unit of the v3 server: own system user, env file, read-only file system except the uploads directory |
| `server/env.example` | every setting the server reads from the environment, names and placeholders only |
| `server/common.sh` | the variables (port, paths, database/role name, v2 bundle directory) and helpers of the two scripts below |
| `server/install.sh` | first install on a host; safe to run again — a re-run keeps what it replaces as `xchange.prev`/`configuration.prev`, like `update.sh` |
| `server/update.sh` | replace binary and configuration, keep the previous ones, print the way back |
| `server/smoke.sh` | checks a running server through a base URL; exit status says pass/fail |
| `server/build.md` | how the binary is built, checked against the target, and copied |
| `nginx/xc-v3-desk.conf.example` | example: desk/admin host proxied to the v3 server |
| `nginx/xc-v3-customer.conf.example` | example: customer host once the kiosk API is served by v3 |
| `nginx/xc-customer.conf` | customer host of phase 1 (Flutter web build + v2 kiosk API) — see the guide at the end of this file |

---

# v3 server (Rust, `api/`)

One process, listening on `127.0.0.1` only, behind nginx. It reads
`api/configuration/base.yaml` plus `<APP_ENVIRONMENT>.yaml`, then environment
variables (`APP_<SECTION>__<KEY>`), and **applies its database migrations every
time it starts** — there is no separate migrate step.

Layout on the host (defaults; all are variables in `server/common.sh`):
On a host that is already installed, the uploads path and the port are read back
from the env file, so a later run without the same overrides does not move them.

| Path | Owner, mode | Content |
|---|---|---|
| `/opt/xc-v3/xchange` | root, 755 | the binary (`xchange.prev`: the one before the last update) |
| `/opt/xc-v3/configuration/` | root, 644 | `api/configuration/*.yaml` |
| `/opt/xc-v3/web/` | root, read-only | pages and static files extracted from the v2 bundle on this host |
| `/etc/xc-v3/env` | root:xc-v3, 640 | settings, including the database password |
| `/var/lib/xc-v3/uploads/` | xc-v3, 700 | uploaded files — the only place the service can write |
| parents the install had to create (e.g. `/var/lib/xc-v3`) | root, 755 | nothing else |
| database `xc_v3`, role `xc_v3` | — | PostgreSQL on the same host |

## 1. Order of a first install

1. **Build and stage** — `server/build.md`, sections 1–3 (build host). The
   build names its commit explicitly (`BUILD_SHA`, `BUILT_AT`) and refuses a
   dirty tree; the same sha goes into the stage's `BUILD` file.
2. **Copy and verify** — `server/build.md`, section 4.
3. **Install** — on the target, as a sudo-capable user (not root):
   ```bash
   cd ~/xc-v3-stage && ./install.sh
   ```
   Override variables on the command line when the defaults do not fit, for
   example `PORT=8094 APP_ENVIRONMENT=staging V2_BUNDLE_DIR=/opt/xc-desk-v2 ./install.sh`.
   It creates the system user, the directories (a directory that already
   exists is never re-owned or re-moded — a parent is left alone, and one of
   the installation's own directories with an unexpected owner or mode stops
   the script; `/var/lib`, `/opt`, `/etc` and the like are refused as targets),
   the env file (database password
   generated on the host, never printed), the role and database, installs the
   binary and configuration, extracts the pages from the v2 bundle, installs and
   starts the unit, and ends with the smoke check. `APP_ENVIRONMENT=production`
   makes the server report channel `release`; anything else reports `staging`.
4. **First owner account** — section 2 below.
5. **nginx** — hand `nginx/xc-v3-desk.conf.example` to the host's owner. Until
   nginx points at the new port nobody outside the host reaches the server.
6. **Smoke check through nginx**, from anywhere:
   ```bash
   deploy/server/smoke.sh https://<desk host> <build sha>
   ```
   and once on the host, to see that the reference-rate source is reachable:
   ```bash
   SMOKE_BOT=1 ./smoke.sh http://127.0.0.1:8094
   ```

Requirements on the target: Ubuntu with systemd, PostgreSQL running locally
with password login on `127.0.0.1`, `curl`, `openssl`, `sha256sum`, and `node`
(only for extracting the pages). The scripts were exercised in a throw-away
Ubuntu 24.04 container with a stubbed `systemctl`; they have **not** been run on
a real host in this form (the two scripts they were cleaned up from were, on
2026-10-06).

## 2. First owner account

The server starts on an empty database and has no sign-up page. The first owner
is created once, on the host, with the `create-owner` sub-command; it reads the
password as one line from standard input and refuses when an owner already
exists.

```bash
read -rs -p 'Owner password: ' XC_OWNER_PW; echo
printf '%s\n' "$XC_OWNER_PW" | sudo -u xc-v3 bash -c \
  'set -a; . /etc/xc-v3/env; set +a; cd /opt/xc-v3 && exec ./xchange create-owner --username "$1"' \
  _ <name>
unset XC_OWNER_PW
```

- `read -rs` does not echo the password, and `printf` is a shell builtin, so the
  password is never on a command line (`ps`) or in the shell history.
- It runs as the service user with the same env file the service uses, so it
  reaches the same database. Expected output: `Owner <name> created`.
- Changing an owner's password later is not designed yet
  (`docs/design/xchange-v3.md`, "First owner").

## 3. Update

Build, stage, copy and verify as for an install, then on the target:

```bash
cd ~/xc-v3-stage && ./update.sh
```

It stops the service; replaces the binary when it differs (keeping the running
one as `xchange.prev`) and the configuration when it differs (keeping the
running one as `configuration.prev` — also when only the configuration
changed); removes every installed `*.yaml` the stage no longer carries, so
the installed set mirrors the stage's (each removed one stays in
`configuration.prev`); starts the service; runs the smoke check (the `build`
reported must be the staged one); and prints the number of applied migrations
before and after. Running it twice with the same stage replaces nothing and
leaves both `.prev` copies as they were.

When the smoke check fails it prints the service log and the rollback commands
and exits non-zero. If a step fails between the stop and the start, it says
that the service is **stopped** and prints the commands that put back what was
saved and start it. It never rolls back by itself.

## 4. Rollback

`update.sh` prints these with the host's real paths, and only the lines for
what that run replaced (a configuration-only update prints no binary line; a
run that replaced nothing says so):

```bash
sudo systemctl stop xc-v3
sudo install -o root -g root -m 755 /opt/xc-v3/xchange.prev /opt/xc-v3/xchange
sudo rm -rf /opt/xc-v3/configuration && sudo cp -a /opt/xc-v3/configuration.prev /opt/xc-v3/configuration
sudo systemctl start xc-v3
./smoke.sh http://127.0.0.1:8094
```

**This is enough only when the update applied no migration.** Migrations are
forward-only and run at start; the migration runner (sqlx, default settings)
refuses to start a binary that does not know a migration already applied to the
database. If `update.sh` reported a different migration count before and after,
going back needs the database restored to its state before the update as well —
and there is no backup procedure yet (next section). This start refusal follows
from sqlx's documented default and was not exercised here.

Rolling the whole cutover back (v3 → v2) is a different thing: the nginx
upstream is pointed back at v2 by the host's owner; v3's database is kept.

## 5. Backups — not written yet

- **Not written yet: database dump** (`xc_v3`; how often, where it is kept, how a
  restore is rehearsed).
- **Not written yet: uploads directory** (`/var/lib/xc-v3/uploads`; must be
  backed up together with the database — rows point at files by name).
- Not written yet: log retention (the service logs to the journal).

Until these exist, an update that applies a migration has no tested way back.

## 6. nginx

The examples in `nginx/` are examples; the host's owner applies them. What the
server depends on, whatever the file looks like in the end:

- `X-Real-IP` set from the connection (`$remote_addr`), overwriting the client's
  value. The server reads only `X-Real-IP` at every door (api/src/presentation/http/client_address.rs):
  all per-address limits (owner sign-in 30/addr, staff sign-in 30/addr, kiosk quotes
  20/min/addr) key on this header. `CF-Connecting-IP` is cleared (`""`) by nginx
  so that an untrusted header sent by a client never reaches downstream.
- Behind Cloudflare, the http-level realip configuration
  (`deploy/nginx/cloudflare-realip.conf.example`, installed as
  `/etc/nginx/conf.d/cloudflare-realip.conf`) is required. Without it,
  `$remote_addr` is a Cloudflare edge address: all visitors share edge IPs,
  one busy minute refuses all kiosk quotes, and 30 failed passwords lock out everyone.
  The host's owner generates `cloudflare-realip.conf` from Cloudflare's published
  IP lists (`https://www.cloudflare.com/ips-v4` and `/ips-v6`) using the command
  in `cloudflare-realip.conf.example`, and keeps the ranges current when Cloudflare
  updates them.
- A host that terminates TLS itself (no Cloudflare) does not use
  `cloudflare-realip.conf`: `$remote_addr` is already the visitor.
- Firewall: the host's port 80 should accept traffic from Cloudflare's ranges
  only. If port 80 is open to direct connections, a direct caller connects directly
  to nginx, so its own address becomes `$remote_addr` and is counted under its own
  per-address limit (there is no way around the rate limit), but the origin server's
  IP is exposed to direct access.
- `Host` passed through and `X-Forwarded-Proto` telling the truth about what the
  browser used (`https` when TLS ends at Cloudflare) — a POST is refused unless
  `Origin` equals `<proto>://<host>`.
- Body size 16 MB on the desk host (the server's upload limit is 15 MB).
- The server's port reachable from nginx only (it listens on `127.0.0.1`).
- Customer host: only the kiosk files and `POST /api/kiosk`.

The examples are written for a host reached through Cloudflare (nginx on port
80, `X-Forwarded-Proto https` fixed); the `$scheme` variant for a host that
terminates TLS itself is in the file as a comment. `nginx -t` passes on both
site examples together with `cloudflare-realip.conf` in a throw-away
`nginx:1.27-alpine` container; that says the syntax is right, not that the file
fits a given host — its owner runs `nginx -t` there before any reload.

---

# Customer Host Deployment Guide (xc-customer.spacing.cloud)

This guide documents deploying the Flutter web kiosk on the customer-facing host (`xc-customer.spacing.cloud`) as specified in `docs/design/xchange-v3.md` §8 (Phase 1).

TLS terminates upstream at Cloudflare, which connects to the customer host over plain HTTP on port 80. Nginx serves the Flutter web SPA assets locally and proxies only `POST /api/kiosk` to the backend server listening on `127.0.0.1:8093`.

---

## 1. Build Command

Build the Flutter web client in release mode:

```bash
cd app && flutter build web --release
```

The output bundle is generated in `app/build/web/`.

---

## 2. Where the Files Go

- **Web Assets**: All files from `app/build/web/` go to `/var/www/xc-customer/web/` on the customer host.
- **Nginx Configuration**: `deploy/nginx/xc-customer.conf` replaces the existing server configuration file that serves `xc-customer.spacing.cloud` under `/etc/nginx/` (for example, `/etc/nginx/conf.d/xc-customer.conf` or `/etc/nginx/conf.d/xc-customer.spacing.cloud.conf`).
- **Backups**: Previous configuration backups are stored outside `/etc/nginx/` under `/var/backups/xc-customer/` so they are never matched by search queries or loaded by Nginx include directives.

---

## 3. How to Apply

1. **Deploy Web Assets**:
   Sync the built assets to the customer host directory:
   ```bash
   rsync -avz --delete app/build/web/ user@xc-customer.spacing.cloud:/var/www/xc-customer/web/
   ```

2. **Locate and Back Up Existing Nginx Configuration**:
   Before copying any file, print every file under `/etc/nginx/` that contains `xc-customer.spacing.cloud`:
   ```bash
   MATCHES=$(grep -rl 'xc-customer.spacing.cloud' /etc/nginx/)
   echo "Matching configuration files:"
   echo "$MATCHES"
   ```
   - **If more than one file matches**: **STOP immediately**. Having multiple files with the same `server_name` causes conflicting definitions. Inspect the files, eliminate duplicates, and resolve by hand before proceeding.
   - **If zero files match** (fresh host): Set target destination to `/etc/nginx/conf.d/xc-customer.conf` and proceed to step 3 without creating a backup.
   - **If exactly one file matches**: Continue with that file as `TARGET_CONF`.

   Create the backup directory outside `/etc/nginx/` and copy the existing configuration to a named, dated backup:
   ```bash
   TARGET_CONF="$MATCHES"
   sudo mkdir -p /var/backups/xc-customer
   BACKUP="/var/backups/xc-customer/$(basename "$TARGET_CONF").$(date +%Y%m%d_%H%M%S)"
   sudo cp "$TARGET_CONF" "$BACKUP"
   echo "Backup created at: $BACKUP"
   ```

3. **Deploy Nginx Configuration**:
   Replace the identified live configuration file with the new configuration, ensuring there is never a second `server` block for the same name:
   ```bash
   sudo cp deploy/nginx/xc-customer.conf "$TARGET_CONF"
   ```

4. **Verify and Reload**:
   Test configuration syntax:
   ```bash
   sudo nginx -t
   ```
   > [!WARNING]
   > A `conflicting server name "xc-customer.spacing.cloud"` warning from `nginx -t` means a second `server` block exists for the same host. You **must stop and undo immediately**, even though `nginx -t` ends with "test is successful".
   
   If `nginx -t` passes with zero warnings or errors, reload nginx:
   ```bash
   sudo systemctl reload nginx
   ```

> [!NOTE]
> Do not apply configuration on any live host without explicit authorization from the system owner.

---

## 4. How to Roll Back

If issues occur with the Flutter web build, roll back by restoring the previous nginx configuration from the backup directory created in Step 2:

1. **Locate Live Configuration File**:
   Find the live configuration file currently serving the host:
   ```bash
   TARGET_CONF=$(grep -rl 'xc-customer.spacing.cloud' /etc/nginx/)
   echo "Target configuration to restore: $TARGET_CONF"
   ```
   Verify that `TARGET_CONF` prints exactly one live file.

2. **Select Desired Backup**:
   List available backups in `/var/backups/xc-customer/` in reverse chronological order:
   ```bash
   ls -lt /var/backups/xc-customer/
   ```
   Pick the newest backup whose content is the one you want to restore. You can verify contents using `grep` or `head` (for example, the v2 configuration does not contain `root /var/www/xc-customer/web` and proxies static assets directly):
   ```bash
   # Check which backups have 'root ' (v3 Flutter) vs proxying (v2):
   grep -c 'root ' /var/backups/xc-customer/*
   # Or inspect file header:
   head -n 20 /var/backups/xc-customer/<filename>
   ```

3. **Restore Backup**:
   Restore the chosen backup file over the live configuration file:
   ```bash
   sudo cp /var/backups/xc-customer/<chosen-backup-file> "$TARGET_CONF"
   ```

4. **Verify and Reload**:
   Test configuration syntax and reload nginx:
   ```bash
   sudo nginx -t
   sudo systemctl reload nginx
   ```

The v2 kiosk files stay on disk and continue to be served through upstream `127.0.0.1:8093`.

---

## 5. How to Confirm

### 5.1 Verify API Proxy (`POST /api/kiosk`)
Run the following curl command to verify the kiosk API proxy:

```bash
curl -s -X POST \
  -H 'Origin: https://xc-customer.spacing.cloud' \
  -H 'X-Xchange-Request: 1' \
  -H 'Content-Type: application/json' \
  -d '{"action":"rates","payload":{}}' \
  https://xc-customer.spacing.cloud/api/kiosk
```

**Expected response**: JSON response containing the `rates` object from the backend server.

### 5.2 Verify Web Kiosk & Route Fallback
Confirm that both `/` and `/kiosk` serve the application and preserve query parameters (such as `?branch=`):

```bash
curl -i https://xc-customer.spacing.cloud/
curl -i https://xc-customer.spacing.cloud/kiosk
curl -i "https://xc-customer.spacing.cloud/kiosk?branch=bkk"
```

**Expected response**: HTTP `200 OK` serving `index.html` with header `Cache-Control: no-cache`.

### 5.3 Verify Method Restriction
Ensure non-POST requests to `/api/kiosk` are rejected without being proxied:

```bash
curl -i -X GET https://xc-customer.spacing.cloud/api/kiosk
```

**Expected response**: HTTP `405 Method Not Allowed`.
