# v2 fixtures

The scripts here drive the **v2 engine** to record what it computes, so the Rust
code can be held to the same answers.

They need `engine.mjs` beside them: lines 5961–6952 of the deployed bundle's
`server.mjs` (`xchange-online@3ffc5a5`), unchanged, plus one line
`export { runEngine, runOperations, initialOperations };`. The bundle is not in
this repository, so the scripts cannot run in CI; their **outputs** are checked
in under each crate's `tests/fixtures/` and are what the tests read.

    node gen_quote.mjs > ../../crates/xc-domain/tests/fixtures/v2_quote_cases.json
