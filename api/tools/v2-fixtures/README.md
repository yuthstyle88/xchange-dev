# v2 fixtures

The scripts here drive the **v2 engine** to record what it computes, so the Rust
code can be held to the same answers.

They need `engine.mjs` beside them: lines 5961–6952 of the deployed bundle's
`server.mjs` (`xchange-online@3ffc5a5`), unchanged, plus one line
`export { runEngine, runOperations, initialOperations };`. The bundle is not in
this repository, so the scripts cannot run in CI; their **outputs** are checked
in under each crate's `tests/fixtures/` and are what the tests read.

    node gen_quote.mjs > ../../crates/xc-kernel/tests/fixtures/v2_quote_cases.json

`gen_compat.mjs` is the exception: it runs the **whole** deployed bundle
(`server.mjs` with `schema.sql` beside it), not the `engine.mjs` slice, and it
needs a scratch PostgreSQL database. It creates the schema `xc_compat_fixture`
there, applies v2's `schema.sql` to it, drops it again for every scenario and
at the end, and touches nothing outside that schema. It writes
`api/tests/fixtures/snapshot/*.json` and `api/tests/fixtures/rpc/*.json` — what
v2 answers on `/api/rpc` and `/api/staff/rpc` — and writes nothing if a step is
answered with a sentence it did not expect.

    node gen_compat.mjs <v2 bundle dir> <postgres url of a scratch database> [out dir]

`out dir` defaults to `../../tests/fixtures`. The reader of these files is
`api/tests/compat_common/mod.rs`.

## What of v2 may be committed here

Lane controller ruling, 2026-10-07 (#228 item 5(c), from the PR 107 review):
- **Allowed:** the text or the shape (e.g. a regex) of a v2 SQL statement that a stand-in store must answer, v2's sentences, and line citations of the bundle. `gen_account.mjs`, `gen_auth_http.mjs`, `gen_file_routes.mjs`, `gen_documents.mjs` and `gen_devices.mjs` already do this.
- **Never:** a v2 function body or any file of the bundle. A script reads the bundle from a path given at run time and slices it in memory.

