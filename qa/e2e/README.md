# xchange end-to-end suite (Playwright)

Design: `docs/design/qa-e2e.md`. One spec file per area under `tests/`, all built on `tests/helpers.ts`.

Runs against a **running** v3 host. The host is prepared with `deploy/server/seed-test-booking.sh`. The owner runs this check, because it signs in with real test passwords.

```bash
cd qa/e2e
bash scripts/reseed.sh             # writes users.json (gitignored) with fresh seeded accounts; keeps your admin entry
npm install
npx playwright install chromium    # once per machine
npx playwright test
```

**Outputs:**
- Screenshots go to `results/`: one per step, plus each app screen in th, en and zh.
- The booking number and every refusal message are printed in the test log.
- `npx playwright show-report` opens the HTML report.

**What each test covers:**

| Test | #1683 criterion |
|---|---|
| `customer books, the ref code is sent, C03 lists and cancels` | 1, 2, 3 (cancel) |
| `teller desk shows the booking` | 4 (desk) |
| `admin booking-slots opens` | 4 (admin, read only; capacity and closure are changed by hand) |
| `app screens in th, en, zh` | #1605 criterion 3 |

**Not automated** (each needs a state the seed doesn't make):
- the refusals `capacityFull`, `slotUnavailable` and `bookingsClosed`;
- the admin capacity change and closure, which write shared data.

Do these by hand and attach the screenshots.

## Specs

| File | Ticket | Tier |
|---|---|---|
| `smoke-public.spec.ts` | QA-S1 | smoke |
| `smoke-app-a.spec.ts`, `smoke-app-b.spec.ts` | QA-S2, QA-S3 | smoke |
| `smoke-admin-money.spec.ts`, `smoke-admin-org.spec.ts`, `smoke-admin-policy.spec.ts` | QA-S4, QA-S5, QA-S6 | smoke |
| `smoke-staff.spec.ts`, `smoke-desk-ops.spec.ts` | QA-S7, QA-S8 | smoke |
| `flow-booking.spec.ts` | QA-F1 | flow |
| `flow-cash.spec.ts` | QA-F3 (runs before `flow-queue`) | flow |
| `flow-queue.spec.ts`, `flow-rates.spec.ts`, `flow-approvals.spec.ts` | QA-F2, QA-F4, QA-F5 | flow |

## For whoever writes a spec

- Never run the suite against a host (E-3). Check with `npx playwright test --list` and `npx tsc -p .`; both work without `users.json`, falling back to `users.example.json`.
- Import only from `./helpers`. Never edit `helpers.ts`; a missing helper goes on your ticket.
