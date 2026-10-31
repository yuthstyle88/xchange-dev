# xchange
XCHANGE money-changer counter system — Rust + PostgreSQL API, Flutter clients

## Running the API tests

Run them with nextest, the way CI does:

```bash
cd api && cargo nextest run -p xchange -E 'test(/pricing_formula/)'
```

`cargo test` with more than one thread gives false failures. `RoleGrants::install` (`crates/xc-auth/src/domain/access/role_grants.rs`) replaces a process-global snapshot, and about 20 test files call it. Under `cargo test`, the tests of one binary share a process, so one test's snapshot leaks into another test's requests (e.g. `roleNotAllowedHere` in `pricing_formula_save_command_test` next to `pricing_formula_save_permission_test`, #564 issuecomment-6538838105). nextest runs each test in its own process. If you must use `cargo test`, add `-- --test-threads=1`.

A pull request's own CI runs guards, not tests (`.github/scripts/api-ci-plan.sh`). The full suite runs on a push to `main` and on a batch PR whose branch is `batch/merge-*`.
