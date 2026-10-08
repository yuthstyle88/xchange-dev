// V1-devices-screen (#404): the label maps (ticket §7 `labels.ts`, §10.1 D14/D15, §10.3 V7).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// This file pins only which codes each map covers. What each label says, in Thai and in English,
// is pinned through the rendered screen in `DevicesScreen.test.tsx`.
import { describe, expect, it } from 'vitest';
import { KIND_LABELS, STATUS_LABELS } from './labels';

/** v2's statuses (D14) and v3's three more (V7, `device_job.rs:61-67`). */
const STATUSES = ['QUEUED', 'SIMULATED_PRINTED', 'COMMAND_SIMULATED', 'FAILED', 'LEASED', 'EXPIRED', 'DRAWER_OPENED'];

/** v2's kinds (D15) and, as event kinds, V7's three. */
const KINDS = [
  'SCAN_IMPORTED',
  'COUNT_CONFIRMED',
  'EXCHANGE_LINKED',
  'PRINT_RECEIPT',
  'DRAWER_REQUEST',
  'DRAWER_MANUAL_OBSERVATION',
  'SIMULATED_PRINTED',
  'COMMAND_SIMULATED',
  'JOB_RETRY',
  'FAILED',
  'SIMULATED_PRINTER_CONNECTION',
  'RECEIPT_IMAGE_ARCHIVED',
  'KIOSK_REQUEST',
  'KIOSK_ACCEPTED',
  'KIOSK_ID_CHECKED',
  'KIOSK_COMPLETED',
  'KIOSK_CURRENCY_CONFIGURED',
  'LEASED',
  'EXPIRED',
  'DRAWER_OPENED',
];

describe('labels (#404 D14, D15, V7)', () => {
  it('STATUS_LABELS has an entry for each job status v3 can store', () => {
    for (const code of STATUSES) {
      expect(Object.prototype.hasOwnProperty.call(STATUS_LABELS, code), code).toBe(true);
    }
  });

  it("KIND_LABELS has an entry for each of v2's kinds and V7's three", () => {
    for (const code of KINDS) {
      expect(Object.prototype.hasOwnProperty.call(KIND_LABELS, code), code).toBe(true);
    }
  });
});
