# Migration Version Allocator and Renumbering Process

**Status:** Adopted (Process note for #1082)  
**Date:** 2026-10-10  
**Authors:** gemini/pool-7, claude/5edf7bb5  
**Refs:** #847 (migration order gate), #1082 (central migration allocator)

---

## 1. Purpose and Overview

In sqlx, database migrations are applied in version order based on their 14-digit timestamp prefix (`YYYYMMDDHHMMSS`). Previously, workers selected migration timestamps locally from `main` at push time. Because multiple PRs can be developed concurrently and `main` is not configured with strict linear branch protection (`required_status_checks.strict = false`), migration versions frequently collided or became stale while PRs were waiting for review or CI.

To eliminate manual renames, merge delays, and version collisions, this document establishes a centralized process for migration slot allocation and merge-time renumbering via `scripts/migration-slot.sh`.

---

## 2. Push-Time Allocation (`next`)

Workers adding a database migration run the allocator script at push time:

```bash
# Get the next available version timestamp
scripts/migration-slot.sh next

# Or generate the complete migration file path directly
scripts/migration-slot.sh next --name <migration_name>
```

- **Ticket conventions:** New tickets that add migrations must stop specifying a fixed, hard-coded timestamp slot. Instead, ticket instructions state:
  > *"run `scripts/migration-slot.sh next` at push time"*
- **Allocation logic:** The script queries `origin/main`'s `api/migrations/` and all open pull requests via `gh pr list --state open --limit 500 --json number,headRefName,files`. It identifies the highest existing version across both `main` and open PRs and steps forward by exactly one calendar-correct hour (`+1h`), handling leap years, month-end, and year-end rollovers.
- **Fail-loud policy:** If `gh` is unavailable or fails, `scripts/migration-slot.sh` fails loudly with a non-zero exit code and an explanatory error on stderr; it never falls back to `main` alone.

---

## 3. Merge-Time Renumbering (`renumber`)

Because pull requests may merge in a different order than they were authored, a PR that was green against `main` at CI time may become stale (`<= origin/main`'s newest migration) by the time it is merged.

The merge batch runs `renumber` immediately before merging any PR that adds a migration:

```bash
scripts/migration-slot.sh renumber <pr-number|branch>
```

- **Requirements:**
  1. The PR branch must be clean (`git status --porcelain` is empty).
  2. The working copy must be checked out to the PR's `headRefName`.
  3. The PR must be currently open.
- **Execution:** The script checks every migration added or renamed since `git merge-base origin/main HEAD` (`--diff-filter=AR`).
  - If all added migrations are already strictly greater than `origin/main`'s highest migration, the script outputs `nothing to renumber` and exits 0 with HEAD unchanged.
  - If any migration has a version `<= origin/main`'s highest version, the script renames each such file via `git mv` to the next slot (calculated above `main`, all other open PRs, and the PR's own migrations that stay above `main`), preserving SQL content byte-for-byte.
  - Each file rename is committed as a single commit with the exact subject:
    ```
    chore(migrations): renumber <old> → <new> at merge time
    ```
    and the script prints `<old> → <new>`.

---

## 4. Renaming and Review Verdicts (Re-bind Rule)

Under **Rule A** (the owner's ruling, 2026-10-10):
- A review `PASS` verdict carries over **only across merges of main**.
- Any commit added after the `PASS` head—including a renumber/rename commit—invalidates the verdict and requires a **re-bind**.
- **Current Rule:** Today, a rename commit after the PASS head requires a re-bind.
- **Open Proposal (undecided):** A proposal is currently open to allow a *pure* rename commit to preserve the PASS verdict without a re-bind, where a pure rename is defined as `git diff -M100% --name-status <old-head> <new-head>` containing only `R100 api/migrations/<old> api/migrations/<new>` lines for files added by the PR.
  > **Note:** This proposal remains **open: the owner decides; never write it as decided**. If and when the owner approves the proposal, the lane controller will record the ruling.

---

## 5. sqlx Semantics and Invariants

Understanding sqlx migration mechanics is critical to avoiding schema divergence:

1. **Out-of-order execution:** sqlx (`0.9.0`) inspects the `_sqlx_migrations` table and applies any migration whose version is missing from that table. It does not reject lower or out-of-order versions (`ignore_missing` defaults to `false`). If an older migration is introduced after a newer one has run, sqlx will attempt to execute it against the current schema.
2. **Missing version boot failure:** sqlx validates applied migrations during boot (`validate_applied_migrations` in `migrator.rs:363-377`). If a version recorded in `_sqlx_migrations` is missing from the binary's embedded migrations, startup fails with `MigrateError::VersionMissing(v)`.
3. **Immutability of applied migrations:**
   > **CONSEQUENCE:** Never renumber a migration that any persistent database has already applied.
   Renumbering is strictly safe only before merging to `main`, while the migration exists solely on an unmerged PR branch. Any developer or transient test database that already ran the branch before renumbering must delete the applied row from `_sqlx_migrations` or be recreated.

---

## 6. Data Migration Statement

This ticket (#1082) and process change contain **no data migration**. It introduces developer and CI process tooling (`scripts/migration-slot.sh`), workflow definitions, and documentation. No database tables, columns, indexes, constraints, or rows are added or modified, and no schema backfill or deployment sequencing is required.
