# Building the v3 server for a host

The server is built on a build host and copied to the target as one binary.
Nothing is compiled on the target. Building is harmless; **copying to a host and
running `install.sh` / `update.sh` there happens only on the owner's order.**

## 1. Build

On the build host, in a clean checkout of the commit to ship:

```bash
git fetch origin
git checkout --detach <sha>

npm --prefix web ci && npm --prefix web run build

cd api
rustc --version                   # rustup follows api/rust-toolchain.toml (1.98.1)

# refuses a dirty tree, and names the commit and the time explicitly
test -z "$(git status --porcelain)" || echo "DIRTY TREE - do not build from it"
test -z "$(git status --porcelain)" &&
  BUILD_SHA="$(git rev-parse HEAD)" BUILT_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    cargo build --release --locked

# the binary must carry exactly this commit
strings target/release/xchange | grep -c "$(git rev-parse HEAD)"    # 1 or more
```

- **Web build before release:** A release build refuses a `web/dist` without `.vite/manifest.json`, and `/health/ready` reports `ui`.
- **Why `BUILD_SHA` and `BUILT_AT` are set by hand:** `api/build.rs` re-runs only
  when one of these two variables or `migrations/` changes, so with a kept
  `target/` or a compiler cache a build that leaves them unset can hand out a
  binary that still names an earlier commit (seen on 2026-10-06; issue #133) —
  setting both forces the re-run and bakes in the commit actually built.
  `/health/ready` reports them as `build` and `builtAt`.
- A dirty tree would carry a sha that is not what was built; the `test` line
  stops the build then (it prints the warning and `cargo` does not run).
- `--locked` refuses to build when `Cargo.lock` would change.
- The shared crates come from the private repository
  `108-Plaza/108-platform-services` at a pinned revision (`api/Cargo.toml`), so
  the build host needs git access to it.

## 2. Check that the binary will run on the target

A binary linked against a newer glibc than the target has does not start. Compare,
on **both** the build host and the target:

```bash
ldd --version | head -n 1         # glibc
openssl version                   # OpenSSL
uname -m                          # architecture
```

and look at what this binary actually asks for (build host):

```bash
ldd target/release/xchange
objdump -T target/release/xchange | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -n 1
```

The highest `GLIBC_x.y` printed must not be newer than the target's glibc, and
the architecture must be the same. At `eba44f5` the lock file has no
`openssl-sys` (TLS is rustls), so `ldd` is expected to list no `libssl` /
`libcrypto`; if a later change makes it list one, the OpenSSL major versions must
match too. The final word is the target itself, after copying (step 4):

```bash
ldd ./xchange | grep 'not found'  # must print nothing
```

(Build host and test host were measured equal on 2026-10-06, see issue #121.
Measure again when either machine is upgraded.)

## 3. Assemble the stage directory

`install.sh` and `update.sh` take everything from one directory:

```bash
STAGE="$HOME/xc-v3-stage"         # not under /tmp: other users can write there
rm -rf "$STAGE" && install -d -m 700 "$STAGE"

cp target/release/xchange            "$STAGE/"
cp -R configuration                  "$STAGE/configuration"
cp -R migrations                     "$STAGE/migrations"
cp tools/extract-v2-pages.mjs        "$STAGE/"
cp ../deploy/server/{common.sh,install.sh,update.sh,smoke.sh,xc-v3.service} "$STAGE/"
git rev-parse HEAD                 > "$STAGE/BUILD"

( cd "$STAGE" && sha256sum xchange configuration/*.yaml migrations/*.sql extract-v2-pages.mjs \
    common.sh install.sh update.sh smoke.sh xc-v3.service BUILD > SHA256SUMS )
cat "$STAGE/SHA256SUMS"
```

`BUILD` holds the same sha that was given to the build as `BUILD_SHA` (both are
`git rev-parse HEAD` of the same clean checkout). The smoke check compares
`/health/ready`'s `build` with it, so an update that left the old binary
running — or a binary that names another commit — fails instead of passing.

No v2 file goes into the stage: the pages are extracted on the target from the
v2 bundle already installed there (`extract-v2-pages.mjs`), and are never copied
to another machine or committed here.

## 4. Copy and verify on both ends

```bash
rsync -a --chmod=D700,F600 "$STAGE/" <user>@<target>:xc-v3-stage/
```

On the target:

```bash
chmod 700 ~/xc-v3-stage && chmod +x ~/xc-v3-stage/{xchange,install.sh,update.sh,smoke.sh}
cd ~/xc-v3-stage && sha256sum -c SHA256SUMS      # every line must say OK
sha256sum xchange                                # same digest as on the build host
ldd ./xchange | grep 'not found'                 # must print nothing
```

`install.sh` / `update.sh` run the checksum comparison again and refuse a stage
that other users could write to. After installing, both print the digest of the
installed `/opt/xc-v3/xchange`; it must be the one from the build host.
