// B03 the desk's `?kiosk=<id>` entry (FE-desk skeleton stub, #564; design xchange-v3-wave3-desk.md §3.6 row K1, §5.3
// `DeskEntryState`, §6 `entries/<name>/entry.ts` found by `entries.ts`'s glob). Pre-fill and identity only: the receipt
// publish half waits on O-1 (`xchange-v3-wave3-desk.md:809`), and completing the request needs its own quote (G7,
// api/src/application/workflows/exchange/create.rs:228-244), which `/api/kiosk get` does not return yet. Filled by its
// own ticket.
import type { DeskEntry } from '../../entries';

export const entry: DeskEntry = {
  param: 'kiosk',
  load() {
    return Promise.reject(new Error('the kiosk entry is not built yet'));
  },
};
