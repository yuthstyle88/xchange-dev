// Specification tests for QU-6 (docs/design/blueprint/queue-tickets.md §4.5, §9.1 row QU-6):
// (d) mount a real staff page AND an admin page through their entries (main.tsx) and see the queue card's lines.
//
// Written before implementation by test-writer (gemini/studio-11, ordered by
// claude/5edf7bb5, Owner's Rule A). Do not edit, delete or weaken.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Me, ViewAnswer } from '../../../shared/api/generated/common';
import type { QueueBoardData } from '../../../shared/api/generated/queue_board';

const BOARD_DATA: QueueBoardData = {
  waiting: [
    {
      id: 'ticket-wait-1',
      displayNumber: 'W042',
      serviceType: 'WALK_IN',
      sequence: 42,
      issuedAt: '2026-10-12T01:00:00.000Z',
    },
  ],
  called: [
    {
      id: 'ticket-called-1',
      displayNumber: 'W041',
      serviceType: 'WALK_IN',
      status: 'SERVING',
      tellerId: 'emp-nok',
      calledAt: '2026-10-12T01:10:00.000Z',
    },
  ],
  myCounter: { status: 'ACTIVE' },
};

const ME_DATA: Me = {
  id: 'emp-nok',
  name: 'Nok',
  roles: [{ role: 'BRANCH_MANAGER', branchId: 'B1' }],
  permissions: ['QueueOperate', 'ShiftOperate', 'CashManage', 'BranchesManage', 'TransactionRead'],
  branch: 'B1',
  branchName: 'Silom',
  branches: [{ id: 'B1', code: 'B1', name: 'Silom' }],
};

function setupMockFetch() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : (input as Request).url;

      if (url.includes('/api/v3/views/me')) {
        const body: ViewAnswer<Me> = {
          version: 1,
          changed: true,
          data: ME_DATA,
        };
        return new Response(JSON.stringify(body), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (url.includes('/api/v3/views/staff_me')) {
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: {
              userId: 'emp-nok',
              name: 'Nok',
              roles: [{ role: 'BRANCH_MANAGER', branchId: 'B1' }],
              branchId: 'B1',
              branchName: 'Silom',
              permissions: ['QueueOperate'],
            },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (url.includes('/api/v3/views/queue_board')) {
        const body: ViewAnswer<QueueBoardData> = {
          version: 1,
          changed: true,
          data: BOARD_DATA,
        };
        return new Response(JSON.stringify(body), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      // Default empty answer for other views/routes
      return new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }),
  );
}

function findQueueCard(): HTMLElement | null {
  return document.querySelector('aside[aria-label="แจ้งเตือนคิวลูกค้า"]');
}

describe('Entries mount and display queue card lines from queue_board (QU-6 item d)', () => {
  let appEl: HTMLDivElement;

  beforeEach(() => {
    document.body.innerHTML = '';
    appEl = document.createElement('div');
    appEl.id = 'app';
    document.body.appendChild(appEl);
    setupMockFetch();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
  });

  // QU-6 (#1354, ruling issuecomment-6118459878) by the test-writer claude/14cc24bd: importing a whole page entry
  // (`main.tsx`) takes 1.4–1.9 s idle and timed out at vitest's default 5 s under load (load average 40–60). An explicit
  // per-test timeout; the `waitFor` conditions are unchanged.
  const ENTRY_MOUNT_TIMEOUT_MS = 20_000;

  it('mounts real staff page through its entry (staff/main.tsx) and sees queue card lines', async () => {
    vi.resetModules();
    await import('../../staff/main');

    await vi.waitFor(() => {
      const card = findQueueCard();
      expect(card, 'queue card must be mounted on staff page').not.toBeNull();

      const badge = card?.querySelector('.queue-card-badge');
      expect(
        badge?.textContent?.trim(),
        'staff queue card must show waiting count from queue_board',
      ).toBe('1');

      const workerSection = card?.querySelector('.queue-card-workers');
      expect(
        workerSection?.textContent,
        'staff queue card must show called ticket from queue_board',
      ).toContain('W041');
    });
  }, ENTRY_MOUNT_TIMEOUT_MS);

  it('mounts real admin page through its entry (admin/main.tsx) and sees queue card lines', async () => {
    vi.resetModules();
    await import('../main');

    await vi.waitFor(() => {
      const card = findQueueCard();
      expect(card, 'queue card must be mounted on admin page').not.toBeNull();

      const badge = card?.querySelector('.queue-card-badge');
      expect(
        badge?.textContent?.trim(),
        'admin queue card must show waiting count from queue_board',
      ).toBe('1');

      const workerSection = card?.querySelector('.queue-card-workers');
      expect(
        workerSection?.textContent,
        'admin queue card must show called ticket from queue_board',
      ).toContain('W041');
    });
  }, ENTRY_MOUNT_TIMEOUT_MS);
});
