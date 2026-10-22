// PH-bookings up-front tests (docs/design/web-public-get.md §4, rows B1–B6). Written before the implementation by
// the XC FE controller's skeleton session (claude/e8020026). Do not edit, delete or weaken these tests; add tests in
// a file of your own. The harness is copied from Bookings.worded.test.tsx (that file is not edited).
//
// Pinned: My Bookings reads the host org through `readHostOrg(api)` and the branch list through `api.publicGet`.
// When either read fails (a refusal, a network failure, or a host answer with no org), the screen shows its load
// error (`.xc-bookings-error` with `error.loadFailed` and the retry button), whether or not the customer has bookings.
// `error.loadFailed` is the sentence `Bookings.test.tsx` #4196 already pins for a failed branch list; this file pins
// the cases that were silent: no bookings ⇒ the empty state was shown over a failed load. The retry re-reads the host.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makeMyBookingItemDto, makeMyBookingsData } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';
import * as table from './i18n';

const BRANCH_ID = 'XC-B01';
const BRANCH_NAME = 'Silom Branch';
const BOOKING_ID = 'BK-alice-001';

const ACTIVE_BOOKING = makeMyBookingItemDto({
  id: BOOKING_ID,
  bookingNo: 'B261014-0024',
  status: 'CONFIRMED',
  branchId: BRANCH_ID,
  slotId: '00000000-0000-0000-0000-000000000001',
  startAt: '2026-10-14T10:00:00Z',
  endAt: '2026-10-14T10:30:00Z',
  qrToken: 'qr-token-alice-12345',
  cancelCutoffAt: '2026-10-14T09:00:00Z',
});

const BRANCHES = {
  branches: [
    {
      id: BRANCH_ID,
      code: BRANCH_ID,
      name: BRANCH_NAME,
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
  ],
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewEnvelope(data: unknown, version = 1, changed = true): Response {
  return json(200, { version, changed, data });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

const NO_BOOKINGS = () => viewEnvelope(makeMyBookingsData({ bookings: [] }));
const SERVER_ERROR = () => json(500, { error: { code: 'serverError', message: 'boom' } });

describe('Customer My Bookings (C03): a failed host or branch-list read is shown, never silent (PH-bookings)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let hostResponder: () => Response | Promise<Response>;
  let branchesResponder: () => Response | Promise<Response>;
  let listResponder: () => Response | Promise<Response>;
  let hostCalls: number;
  let fetchStub: (input: string, init?: RequestInit) => Promise<Response>;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    vi.setSystemTime(new Date('2026-10-14T08:00:00Z'));

    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);

    hostCalls = 0;
    hostResponder = () => json(200, { org: 'XC' });
    branchesResponder = () => json(200, BRANCHES);
    listResponder = () => viewEnvelope(makeMyBookingsData({ bookings: [{ ...ACTIVE_BOOKING }] }));

    fetchStub = vi.fn(async (input: string): Promise<Response> => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') {
        hostCalls += 1;
        return hostResponder();
      }
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return branchesResponder();
      }
      if (u.pathname === '/api/v3/views/my_bookings') {
        return listResponder();
      }
      return json(404, { error: { code: 'notFound', message: u.pathname } });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  async function mountScreen(): Promise<void> {
    dispose?.();
    host.innerHTML = '';
    dispose = render(() => <BookingsScreen fetch={fetchStub} />, host);
    await flush();
  }

  const errorBox = () => host.querySelector('.xc-bookings-error');

  it('B1: host 500 with a booking shows the load error and the retry (unchanged)', async () => {
    hostResponder = SERVER_ERROR;
    await mountScreen();

    expect(errorBox(), 'the load error is shown').not.toBeNull();
    expect(errorBox()?.textContent).toContain(table.th['error.loadFailed']);
    expect(host.querySelector('[data-action="retry"]')).not.toBeNull();
  });

  it('B2: host 500 with no bookings shows the load error, not the empty state; retry re-reads the host and recovers', async () => {
    hostResponder = SERVER_ERROR;
    listResponder = NO_BOOKINGS;
    await mountScreen();

    expect(errorBox(), 'the load error is shown').not.toBeNull();
    expect(errorBox()?.textContent).toContain(table.th['error.loadFailed']);
    expect(host.querySelector('.xc-bookings-empty'), 'no empty state over a failed load').toBeNull();

    const before = hostCalls;
    hostResponder = () => json(200, { org: 'XC' });
    (host.querySelector('[data-action="retry"]') as HTMLButtonElement).click();
    await flush();

    expect(hostCalls, 'the retry re-reads the host').toBe(before + 1);
    expect(errorBox()).toBeNull();
    expect(host.querySelector('.xc-bookings-empty')).not.toBeNull();
  });

  it('B3: a network failure on the host read, with no bookings, shows the load error', async () => {
    hostResponder = () => {
      throw new TypeError('Failed to fetch');
    };
    listResponder = NO_BOOKINGS;
    await mountScreen();

    expect(errorBox()).not.toBeNull();
    expect(errorBox()?.textContent).toContain(table.th['error.loadFailed']);
    expect(host.querySelector('.xc-bookings-empty')).toBeNull();
    expect(host.textContent).not.toContain('Failed to fetch');
  });

  it('B4: a host answer with no org, with no bookings, shows the load error', async () => {
    hostResponder = () => json(200, {});
    listResponder = NO_BOOKINGS;
    await mountScreen();

    expect(errorBox()).not.toBeNull();
    expect(errorBox()?.textContent).toContain(table.th['error.loadFailed']);
    expect(host.querySelector('.xc-bookings-empty')).toBeNull();
  });

  it('B5: a branch-list 500 after a good host read, with no bookings, shows the load error', async () => {
    branchesResponder = SERVER_ERROR;
    listResponder = NO_BOOKINGS;
    await mountScreen();

    expect(errorBox()).not.toBeNull();
    expect(errorBox()?.textContent).toContain(table.th['error.loadFailed']);
    expect(host.querySelector('.xc-bookings-empty')).toBeNull();
  });

  it('B6: a good host and branch read shows the booking with its branch name and no error, one host read', async () => {
    await mountScreen();

    expect(errorBox()).toBeNull();
    expect(host.querySelector(`[data-booking-id="${BOOKING_ID}"]`)).not.toBeNull();
    expect(host.textContent).toContain(BRANCH_NAME);
    expect(hostCalls).toBe(1);
  });
});
