# Booking end-to-end check (#1683) and app screenshots (#1605)

Runs against a **running** v3 host. The host is prepared with `deploy/server/seed-test-booking.sh`. The owner runs this check, because it signs in with real test passwords.

```bash
cd qa/booking-e2e
cp users.example.json users.json   # fill in the passwords; users.json is gitignored
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
