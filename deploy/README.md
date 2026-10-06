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
