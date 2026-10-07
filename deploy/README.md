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
| `server/backup.sh` | creates one backup set (database dump + uploads archive), prunes to newest 7, optional off-host copy |
| `server/restore.sh` | rehearses a restore into a scratch database and scratch uploads directory, refusing live ones |
| `server/xc-v3-backup.service` | oneshot systemd unit running backup.sh under hardening |
| `server/xc-v3-backup.timer` | daily systemd timer running xc-v3-backup.service with Persistent=true |
| `server/journald-xc-v3.conf` | example drop-in for systemd-journald log retention |
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
5. **nginx and origin TLS** — hand `deploy/nginx/xc-v3-desk.conf.example` and the
   snippet examples (`deploy/nginx/snippets/`) to the host's owner. Nginx listens
   on both port 80 and port 443 with origin TLS (Let's Encrypt certificates issued
   via certbot webroot). Plain HTTP on port 80 redirects to HTTPS inside application
   locations, leaving `/.well-known/acme-challenge/` answering on port 80 for ACME
   renewals (section 6 below). Until nginx points at the new port nobody outside the
   host reaches the server.
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

Before stopping the service, it takes a backup of the database and uploads directory
by running `sudo systemctl start xc-v3-backup.service`. It strictly refuses to
update if `xc-v3-backup.service` is not installed (pointing to §5 below, per BK).
It checks that a new backup set was created under `$XC_BACKUP_DIR`, verifies
that both the dump and uploads archive are age-encrypted (starting with
`age-encryption.org/v1`; `backup.sh` already verified the plain dump with
`pg_restore --list` ensuring `_sqlx_migrations` is present before encrypting),
and only then stops the service.

It then replaces the binary when it differs (keeping the running
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
going back needs the database restored to its state before the update as well,
using the backup set `update.sh` created and named just before the update
(`$XC_BACKUP_DIR/<stamp>/`):
- Rehearsal:
  ```bash
  sudo XC_BACKUP_IDENTITY=<identity> /opt/xc-v3/restore.sh /var/backups/xc-v3/<stamp> <scratch-database> <scratch-uploads-dir>
  ```
- Real restore: see Section 5 "Real restore". This start refusal follows
from sqlx's documented default and was not exercised here.

Rolling the whole cutover back (v3 → v2) is a different thing: the nginx
upstream is pointed back at v2 by the host's owner; v3's database is kept.

## 5. Backups and log retention

Every backup run creates one **set** under `$XC_BACKUP_DIR` (default `/var/backups/xc-v3`), named by a UTC timestamp `<stamp>/` (format `YYYYMMDDTHHMMSSZ`), containing:
- `db-<stamp>.dump.age` — age-encrypted PostgreSQL custom-format archive (`pg_dump -Fc` of `APP_DATABASE__DATABASE_NAME`).
- `uploads-<stamp>.tar.gz.age` — age-encrypted gzip-compressed archive (`tar -czf`) of `APP_FILES__DIR`.

Both the backup directory and each set directory are mode `0700` owned by `root:root`. The set is created in a temporary working directory `.work-<stamp>` and renamed to `<stamp>` only when both pieces exit 0. After each successful run, local sets are pruned so only the newest `XC_BACKUP_KEEP` sets (default 7) remain. If configured, `XC_BACKUP_OFFHOST` mirrors the local sets to a remote host via `rsync`.

### Installing the backup units

1. Install `age` and copy the scripts and units to the host:
   ```bash
   sudo apt update && sudo apt install -y age
   sudo install -m 0755 server/backup.sh /opt/xc-v3/backup.sh
   sudo install -m 0755 server/restore.sh /opt/xc-v3/restore.sh
   sudo install -m 0644 server/xc-v3-backup.service /etc/systemd/system/xc-v3-backup.service
   sudo install -m 0644 server/xc-v3-backup.timer /etc/systemd/system/xc-v3-backup.timer
   sudo systemctl daemon-reload
   ```

2. **Configure age encryption recipients**:
   The backup identity is generated and kept off the server by the owner; only the public recipient key is stored on the server.
   - On the owner's machine, generate the identity with `age-keygen` and keep two copies apart (e.g. offline storage and password manager):
     ```bash
     age-keygen -o xc-backup-identity.txt
     ```
   - Extract the public recipient line (`age1...`):
     ```bash
     age-keygen -y xc-backup-identity.txt
     ```
   - On the server, place the `age1...` recipient in `/etc/xc-v3/backup.age-recipients` (root:root, mode 0644):
     ```bash
     sudo sh -c 'echo "age1..." > /etc/xc-v3/backup.age-recipients'
     sudo chown root:root /etc/xc-v3/backup.age-recipients
     sudo chmod 0644 /etc/xc-v3/backup.age-recipients
     ```

3. **Create the backup directory before enabling the timer** (the unit's `ReadWritePaths=` requires it to exist beforehand):
   ```bash
   sudo install -d -m 0700 -o root -g root /var/backups/xc-v3
   ```
   If `/etc/xc-v3/env` overrides `XC_BACKUP_DIR`, use that path instead (and update `ReadWritePaths=` in `xc-v3-backup.service`).

4. If off-host backups are used, generate an SSH key, configure the remote target, and record the remote host key before enabling the timer:
   ```bash
   # Generate dedicated SSH key (root-owned, mode 0600)
   sudo ssh-keygen -t ed25519 -N "" -f /etc/xc-v3/backup-offhost.key
   sudo chmod 0600 /etc/xc-v3/backup-offhost.key

   # On the off-host backup server:
   # Authorize /etc/xc-v3/backup-offhost.key.pub in ~<offhost-user>/.ssh/authorized_keys
   # for a dedicated user constrained so it can write only the /data target directory.

   # In /etc/xc-v3/env:
   # Set XC_BACKUP_OFFHOST=<offhost-user>@<offhost-host>:/data/<path>

   # Ensure /root/.ssh exists with mode 0700 before scanning keys:
   sudo install -d -m 0700 /root/.ssh

   # Scan and record the off-host machine's host key into root's known_hosts:
   sudo ssh-keyscan -t ed25519 <offhost-host> | sudo tee -a /root/.ssh/known_hosts

   # Verify fingerprints match before trusting the scanned key:
   # On the off-host machine:
   ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
   # On the v3 host:
   sudo ssh-keygen -lf /root/.ssh/known_hosts
   # If the fingerprint does not match the one shown on the off-host machine, run sudo ssh-keygen -R <offhost-host> -f /root/.ssh/known_hosts and stop (do not proceed to the BatchMode step).

   # Verify the SSH connection succeeds non-interactively:
   sudo ssh -i /etc/xc-v3/backup-offhost.key -o BatchMode=yes <offhost-user>@<offhost-host> true
   ```
   > [!NOTE]
   > Why this step is required: `xc-v3-backup.service` runs `backup.sh` non-interactively (`BatchMode=yes`) under systemd hardening with `ProtectHome=read-only`. If the off-host host key is not already present in `/root/.ssh/known_hosts`, SSH cannot prompt the operator and cannot write the key to `/root/.ssh/known_hosts` at runtime, causing every daily off-host backup run to fail with `Host key verification failed`.

   **One-time owner step for existing off-host plain files:**
   Because rsync's include filter sends only `*.age` files and protects excluded target files, any plain files from backups taken before BK-ENC remain on the off-host target until removed manually:
   ```bash
   # On the off-host machine, list plain files in the target directory:
   find <target> -type f ! -name '*.age' -print

   # After reading the list and confirming, delete them:
   find <target> -type f ! -name '*.age' -delete
   ```

5. Enable and start the timer:
   ```bash
   sudo systemctl enable --now xc-v3-backup.timer
   ```

6. To run a backup by hand at any time:
   ```bash
   sudo systemctl start xc-v3-backup.service
   ```
   Check the output in the journal: `journalctl -u xc-v3-backup.service`. The last line of stdout is the path to the completed set.

### Verifying a backup on the owner's machine

The owner can check that a backup set is valid and that their identity key works directly on their own machine, without needing server access:
```bash
age -d -i <identity> db-<stamp>.dump.age | pg_restore --list
age -d -i <identity> uploads-<stamp>.tar.gz.age | tar -tzf -
```
The first command checks that `_sqlx_migrations` is listed in the archive table of contents; the second lists the archived files.

### Rehearsing a restore

To verify a backup set on the server without touching live data, run `restore.sh`. The owner brings the identity temporarily to tmpfs (`/run/xc-v3-restore/identity`, mode 0600) and passes it in `XC_BACKUP_IDENTITY`:
```bash
sudo install -d -m 0700 /run/xc-v3-restore
sudo install -m 0600 <identity-file> /run/xc-v3-restore/identity
sudo XC_BACKUP_IDENTITY=/run/xc-v3-restore/identity /opt/xc-v3/restore.sh /var/backups/xc-v3/<stamp> <scratch-database> <scratch-uploads-dir>
sudo rm -rf /run/xc-v3-restore
```
`restore.sh` strictly refuses the live database name (or case variations) and refuses the live uploads directory or paths inside it. It creates the scratch database as `postgres`, decrypts the dump with `age -d -i "$XC_BACKUP_IDENTITY"` into `pg_restore` via stdin, decrypts and extracts uploads, and prints cleanup instructions. On failure, it drops the scratch database and cleans up the scratch directory automatically.

To remove scratch rehearsal resources:
```bash
sudo runuser -u postgres -- dropdb <scratch-database>
sudo rm -rf <scratch-uploads-dir>
```

### Real restore (disaster recovery)

A real restore replaces live data and must be performed with the service stopped, decrypting with `age -d`:
1. Place identity on tmpfs:
   ```bash
   sudo install -d -m 0700 /run/xc-v3-restore
   sudo install -m 0600 <identity-file> /run/xc-v3-restore/identity
   ```
2. Stop the server:
   ```bash
   sudo systemctl stop xc-v3.service
   ```
3. Restore database:
   ```bash
   sudo runuser -u postgres -- dropdb xc_v3
   sudo runuser -u postgres -- createdb xc_v3
   sudo sh -c 'age -d -i /run/xc-v3-restore/identity /var/backups/xc-v3/<stamp>/db-<stamp>.dump.age | runuser -u postgres -- pg_restore --no-owner -d xc_v3'
   ```
4. Restore uploads:
   ```bash
   # Clear existing uploads without relying on caller shell glob expansion:
   sudo find /var/lib/xc-v3/uploads -mindepth 1 -delete
   sudo sh -c 'age -d -i /run/xc-v3-restore/identity /var/backups/xc-v3/<stamp>/uploads-<stamp>.tar.gz.age | tar -xzf - -C /var/lib/xc-v3/uploads'
   sudo chown -R xc-v3:xc-v3 /var/lib/xc-v3/uploads
   ```
   > [!NOTE]
   > Why `find ... -delete`: A plain `sudo rm -rf /var/lib/xc-v3/uploads/*` expands the `*` glob in the operator's shell before `sudo` executes. Because `/var/lib/xc-v3/uploads` is mode `0700` owned by `xc-v3:xc-v3`, an unprivileged operator's shell cannot list the directory, leaving the `*` pattern unexpanded so `rm` removes nothing. Using `sudo find ... -mindepth 1 -delete` ensures existing files are removed before the archive is extracted.
5. Restart the server and remove the identity from tmpfs:
   ```bash
   sudo systemctl start xc-v3.service
   sudo rm -rf /run/xc-v3-restore
   ```

### Log retention

The server logs to `systemd-journald`. To bound journal log retention host-wide:
```bash
sudo install -d -m 0755 /etc/systemd/journald.conf.d
sudo cp server/journald-xc-v3.conf /etc/systemd/journald.conf.d/xc-v3.conf
sudo systemctl restart systemd-journald
```

## 6. nginx and origin TLS

The examples in `deploy/nginx/` and `deploy/nginx/snippets/` are examples; the
host's owner applies them. What the server depends on:

- **Origin TLS (443 + 80)**: Each host listens on both port 80 and port 443
  (`listen 80; listen [::]:80; listen 443 ssl http2; listen [::]:443 ssl http2;`).
  Origin certificates are issued via Let's Encrypt using certbot with the webroot
  authenticator (`/var/www/letsencrypt`).
- **Snippets**: Shared configuration snippets are placed in `/etc/nginx/snippets/`:
  - `acme-challenge.conf` (`deploy/nginx/snippets/acme-challenge.conf.example`):
    serves `location ^~ /.well-known/acme-challenge/` from webroot `/var/www/letsencrypt`
    with `auth_basic off;` and `default_type text/plain;`.
  - `ssl-params.conf` (`deploy/nginx/snippets/ssl-params.conf.example`):
    enforces `ssl_protocols TLSv1.2 TLSv1.3;`, disables `ssl_session_tickets`, and sets
    shared session cache and cipher preferences.
- **Port 80 redirect inside locations**: Port 80 never serves application traffic
  and always serves ACME HTTP-01 challenges. Because an `if` / `return` directive at
  server level runs in the server rewrite phase before location selection and would
  redirect ACME challenge requests, the HTTPS redirect is written **inside the
  locations**, never at server level:
  `if ($scheme = http) { return 301 https://$host$request_uri; }` as the first line
  of `location /` and of every other location serving or proxying application traffic
  (desk: `location /api/`; customer: `location = /api/kiosk` and the cache location).
- **HSTS**: `add_header Strict-Transport-Security "max-age=31536000" always;` is set
  at server level (without `includeSubDomains` or `preload`). Any location that
  defines its own `add_header` (such as the customer static cache location) repeats
  this directive, as nginx does not inherit parent `add_header` into locations with their
  own header directives.
- `proxy_set_header X-Forwarded-Proto $scheme;`: Passes `https` on TLS connections.
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
- Firewall: the host's ports 80 and 443 should accept traffic from Cloudflare's
  ranges only.
- `Host` passed through and `X-Forwarded-Proto` telling the truth about what the
  browser used (`https` when TLS ends at Cloudflare) — a POST is refused unless
  `Origin` equals `<proto>://<host>`.
- Body size 16 MB on the desk host (the server's upload limit is 15 MB).
- The server's port reachable from nginx only (it listens on `127.0.0.1`).
- Customer host: only the kiosk files and `POST /api/kiosk`.

### Certbot Webroot Setup Order

On a fresh host, certificates and origin TLS are installed in this order:

1. **Snippets and the port-80 block**:
   Create webroot directory `/var/www/letsencrypt`:
   ```bash
   sudo mkdir -p /var/www/letsencrypt
   ```
   Install snippets into `/etc/nginx/snippets/`:
   ```bash
   sudo mkdir -p /etc/nginx/snippets
   sudo cp deploy/nginx/snippets/acme-challenge.conf.example /etc/nginx/snippets/acme-challenge.conf
   sudo cp deploy/nginx/snippets/ssl-params.conf.example /etc/nginx/snippets/ssl-params.conf
   ```
   Configure the vhost with listeners for port 80 only, including `snippets/acme-challenge.conf;`
   and `location /`. Reload nginx (`sudo nginx -t && sudo systemctl reload nginx`).
2. **Issue certificate**:
   Run certbot with the webroot authenticator and deploy hook:
   ```bash
   sudo certbot certonly --webroot -w /var/www/letsencrypt -d <name> --deploy-hook "systemctl reload nginx"
   ```
3. **Dry-run verification**:
   Verify automated renewal works:
   ```bash
   sudo certbot renew --dry-run
   ```
4. **Enable origin TLS**:
   Update the server block to add `listen 443 ssl http2;`, `listen [::]:443 ssl http2;`,
   `include snippets/ssl-params.conf;`, the certificate paths (`ssl_certificate /etc/letsencrypt/live/<name>/fullchain.pem;`
   and `ssl_certificate_key /etc/letsencrypt/live/<name>/privkey.pem;`), server-level HSTS, and the per-location
   HTTPS redirects (`if ($scheme = http) { return 301 https://$host$request_uri; }`).
   Test and reload:
   ```bash
   sudo nginx -t && sudo systemctl reload nginx
   ```

### Owner Checklist (Design §7c)

Order matters:

1. *(done on `do-sgp1`, 2026-10-07)* **Certificates and origin TLS**: Certificates issued via certbot webroot, `listen 443 ssl` on every vhost, and the two snippets included.
2. **Cloudflare SSL/TLS mode**: In Cloudflare dashboard → SSL/TLS, set encryption mode to **Full (strict)**. Note that the dashboard SSL/TLS encryption mode is zone-wide: switching to Full (strict) zone-wide is safe only because every proxied vhost on this host answers on 443 with a valid origin certificate; if another name in the shared zone lives elsewhere without a valid certificate, use a per-hostname Configuration Rule to enforce Full (strict) for the xc hostnames alone. Verify every xc name through Cloudflare: `curl -sI https://<name>/healthz` (xc-v3) or `/` answers 200 with no 525/526 errors.
3. **Lock origin to Cloudflare**: Lock down direct origin access using Authenticated Origin Pulls (AOP: Cloudflare presents a client certificate; nginx `ssl_client_certificate <Cloudflare origin-pull CA>;` + `ssl_verify_client on;` on 443) together with a host firewall admitting ports 80 and 443 from Cloudflare's published IP ranges only (`https://www.cloudflare.com/ips-v4` and `/ips-v6`). Use AOP together with the Cloudflare IP allowlist (not either alone), because the zone-level Authenticated Origin Pulls certificate is shared by every Cloudflare customer, so AOP alone admits requests from any Cloudflare account. Every xc hostname must remain proxied (orange cloud in Cloudflare DNS; never DNS-only): a DNS-only name would send Let's Encrypt renewal checks directly to the origin, which the allowlist will refuse — and switching a name to DNS-only later silently breaks certbot renewal (90 days on). After applying the lock, confirm `sudo certbot renew --dry-run` succeeds for every name.
4. **Port 80 redirect-only**: Verify the TLS-ORIGIN nginx files with the per-location redirect `if ($scheme = http) { return 301 https://$host$request_uri; }` inside `location /` and application locations (never at server level). Run `sudo nginx -t && sudo systemctl reload nginx`. On the origin host, verify that `curl -s -o /dev/null -w '%{http_code}' -H 'Host: <name>' http://127.0.0.1/.well-known/acme-challenge/<file>` answers `200`, while through Cloudflare `curl -s -o /dev/null -w '%{http_code} %{redirect_url}\n' http://<name>/` answers `301 https://<name>/`. Confirm `sudo certbot renew --dry-run` still succeeds.
5. **HSTS and Edge Certificates**: Verify `curl -sI https://<name>/ | grep -i strict-transport-security` returns `max-age=31536000`. In Cloudflare dashboard (SSL/TLS → Edge Certificates), toggle **Always Use HTTPS** to ON and set **Minimum TLS Version** to 1.2.

---

# Customer Host Deployment Guide (xc-customer.spacing.cloud)

This guide documents deploying the Flutter web kiosk on the customer-facing host (`xc-customer.spacing.cloud`) as specified in `docs/design/xchange-v3.md` §8 (Phase 1).

Cloudflare connects to the customer host over HTTPS (origin TLS on port 443 with Let's Encrypt certificates), with port 80 redirecting all non-ACME traffic to HTTPS. Nginx serves the Flutter web SPA assets locally and proxies only `POST /api/kiosk` to the backend server listening on `127.0.0.1:8093`.

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

3. **Deploy Nginx Configuration and Snippets**:
   Ensure snippets exist in `/etc/nginx/snippets/`:
   ```bash
   sudo mkdir -p /etc/nginx/snippets
   sudo cp deploy/nginx/snippets/acme-challenge.conf.example /etc/nginx/snippets/acme-challenge.conf
   sudo cp deploy/nginx/snippets/ssl-params.conf.example /etc/nginx/snippets/ssl-params.conf
   ```
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
