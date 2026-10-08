# XCHANGE Counter Device Agent (`xc-device-agent`)

The `xc-device-agent` is a lightweight background daemon deployed on counter workstations (macOS and Linux). It securely polls the XCHANGE cloud server for drawer kick jobs under an active lease, triggers the physical cash drawer via a local receipt printer (CUPS or raw TCP), and reports the outcome back with audit events.

---

## 1. Artifacts and Checksums

Release builds produce two tarballs, each accompanied by a SHA-256 checksum file:

- **macOS (Universal Binary)**: `xc-device-agent-<version>-<short_sha>-universal-apple-darwin.tar.gz` and `.sha256`
  Contains a universal Mach-O binary supporting both `arm64` (Apple Silicon) and `x86_64` (Intel).
- **Linux (x86_64)**: `xc-device-agent-<version>-<short_sha>-x86_64-unknown-linux-musl.tar.gz` and `.sha256`
  A fully static musl binary (`x86_64-unknown-linux-musl`) with no external libc or OpenSSL dependency, compatible with any shop Linux system regardless of glibc version.

### Verifying Checksums and Linkage

Download the tarball and its `.sha256` file into the same directory, then verify:

```bash
# On macOS / Linux
shasum -a 256 -c xc-device-agent-*.tar.gz.sha256
```

Each verified file must print `OK`. Extract the binary:

```bash
tar -xzf xc-device-agent-*.tar.gz
chmod +x xc-device-agent
```

On macOS, you can confirm the universal binary architectures:

```bash
lipo -archs xc-device-agent
# Output: x86_64 arm64
```

On Linux, you can confirm the binary is statically linked:

```bash
file xc-device-agent
# Output: ... statically linked (or static-pie linked)
ldd xc-device-agent
# Output: not a dynamic executable
```

---

## 2. Pairing and Setup (Owner Desk)

To pair a counter machine with a shop branch:

1. As the shop owner, navigate to:
   ```
   https://<desk-host>/_xc/device-agents
   ```
2. Select the branch and click **Add Agent** (or generate a **New Code** if re-pairing).
3. The system generates a 10-character Crockford Base32 pairing code (formatted as `XXXXX-XXXXX`, valid for 1 hour).
4. Run the `install` command on the counter machine using this code.

---

## 3. Installation

The `install` command exchanges the one-time code for a long-lived bearer token (`xcda_...`), saves configuration and credentials to disk, and configures the background service.

### A. USB Receipt Printer (via CUPS)

Ensure the printer is configured in CUPS (e.g. queue name `POS80_`):

```bash
sudo ./xc-device-agent install \
  --server "https://api.yourshop.com" \
  --code "ABCDE-FGHJK" \
  --cups-queue "POS80_"
```

### B. Network Receipt Printer (via TCP Port 9100)

For printers accessible over LAN on raw port 9100:

```bash
sudo ./xc-device-agent install \
  --server "https://api.yourshop.com" \
  --code "ABCDE-FGHJK" \
  --tcp "192.168.1.100:9100"
```

### Optional Flags

- `--channel <CHANNEL>`: Update channel (`stable` by default).
- `--no-service`: Perform pairing and write local credentials without installing system service units (useful for testing and containerized runs).

---

## 4. Operating System Notes

### macOS

1. Copy the universal binary to a system path (e.g. `/usr/local/bin`):
   ```bash
   sudo cp xc-device-agent /usr/local/bin/
   ```
2. **Quarantine attribute**: Because the binary is not yet Apple-notarized for early shop deployments, macOS Gatekeeper may quarantine downloaded binaries. Remove the quarantine attribute:
   ```bash
   sudo xattr -d com.apple.quarantine /usr/local/bin/xc-device-agent 2>/dev/null || true
   ```
3. Configuration and token are stored in `/Library/Application Support/xc-device-agent` (or `~/.config/xc-device-agent` for user installs).
4. When installed as a service, a LaunchDaemon/LaunchAgent plist is registered.

### Linux

1. Ensure CUPS client utilities (`lp`, `lpstat`) are installed if using USB printer queues.
2. The agent runs under the dedicated system user `xc-agent`. Ensure this user exists and belongs to the `lp` group so it can submit print/drawer jobs:
   ```bash
   sudo useradd -r -s /usr/sbin/nologin -G lp xc-agent || sudo usermod -aG lp xc-agent
   ```
3. Configuration and credentials are saved in `/etc/xc-device-agent` with restrictive permissions (`0600`, owned by `xc-agent`).
4. When installed as a service, a systemd service unit (`/etc/systemd/system/xc-device-agent.service`) is created and started.

---

## 5. Daily Operations and Lifecycle

### Check Status

Inspect the current pairing, configuration, and registered printer target:

```bash
xc-device-agent status
```

Prints the server URL, assigned agent ID, branch ID, agent name, printer transport/target, and token presence.

### Running in Foreground

To test or troubleshoot in the console:

```bash
xc-device-agent run
```

### Uninstallation

To remove the background service and delete local pairing credentials:

```bash
sudo xc-device-agent uninstall
```

### Re-pairing and Revocation

- **Re-pairing (`newCode`)**: If an agent's credentials are lost or revoked, the owner generates a new code on the `/_xc/device-agents` page and runs `install` again. The new token replaces the previous credentials.
- **Revocation**: The owner can revoke an agent directly from `/_xc/device-agents`. Once revoked, the agent will receive HTTP 401 `AGENT_TOKEN_INVALID` on its next poll.
- **"re-pair needed" in logs**: When the agent encounters a fatal authentication failure (token revoked or replaced), it logs `re-pair needed` and ceases polling until re-installed with a new pairing code.

---

## 6. Security and Privacy

- **Token Protection**: The bearer token (`xcda_...`) is stored in a file mode `0600` accessible only by the service user.
- **Zero Log Leakage**: The bearer token is **never printed or logged** by `xc-device-agent` or the server. Handlers and runner logs redact or omit authorization credentials in all debug and error traces.
- **Localhost Rule**: The agent refuses non-TLS connections unless connecting to loopback (`127.0.0.1` / `localhost`).
