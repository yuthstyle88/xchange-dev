// FE-T1b #5852 (design docs/design/fe-triage-1101.md §3 W4; triage T1 on #564). Written before the implementation by
// the FE sub-controller claude/01TxaRJG. Do not edit, delete or weaken; add tests in a file of your own.
//
// [required] by these tests:
// - W4: a CHECKED_IN or IN_SERVICE row shows `button[data-action="cancel"]` (the existing cancel dialog, reason
//   required). Nothing else is added: IN_SERVICE shows cancel only; CHECKED_IN keeps its reissue QR and gets no
//   check-in or reschedule. The server allows a teller cancel with a reason from both states
//   (`api/crates/xc-booking/src/domain/booking/booking_machine.rs:41-51`, `docs/design/blueprint/booking-slots.md:192`).
// - Confirm with a reason sends exactly one `POST /api/bookings/{id}/cancel` whose body carries the trimmed reason.
// - A blank reason sends nothing and shows the `reasonRequired` text (`./i18n.ts`).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { withBookingSlots } from './booking-slots.testing';
import { en } from './i18n';

const BRANCH_ID = 'XC-B01';
const SLOT_ID = '00000000-0000-0000-0000-000000000001';

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: BRANCH_ID })],
  branch: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const BOOKINGS_DATA = makeDeskBookingsData({
  items: (['CHECKED_IN', 'IN_SERVICE'] as const).map((status, i) =>
    makeDeskBookingItemDto({
      id: `bk-${status.toLowerCase()}`,
      bookingNo: `B261101-000${i + 1}`,
      status,
      customerDisplayName: `Customer ${status}`,
      phoneLast4: `${2000 + i}`,
      readiness: 'unknown',
      ticketNumber: `B00${i + 1}`,
      slotId: SLOT_ID,
      startAt: '2026-11-01T09:00:00Z',
      endAt: '2026-11-01T09:30:00Z',
    }),
  ),
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}
const viewEnvelope = (data: unknown) => json(200, { version: 1, changed: true, data });

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
}

type Post = { path: string; body: any };

describe('DeskBookingsScreen: cancel a checked-in or in-service booking (FE-T1b W4)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let posts: Post[];

  beforeAll(() => warmModules(() => import('./DeskBookingsScreen')), 60_000);

  beforeEach(() => {
    setAudience('staff');
    setLang('en');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    posts = [];

    vi.stubGlobal(
      'fetch',
      withBookingSlots(
        vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
          const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
          const u = new URL(rawUrl, 'http://localhost');
          const method = (init?.method ?? 'GET').toUpperCase();
          if (u.pathname === '/api/v3/views/me') return viewEnvelope(STAFF_TELLER);
          if (u.pathname === '/api/v3/views/desk_bookings') return viewEnvelope(BOOKINGS_DATA);
          if (method === 'POST') {
            posts.push({ path: u.pathname, body: init?.body ? JSON.parse(String(init.body)) : null });
            const m = /^\/api\/bookings\/([^/]+)\/cancel$/.exec(u.pathname);
            if (m) return json(200, { id: m[1], status: 'CANCELLED', bookingNo: 'B261101-0002' });
          }
          return json(404, { error: { code: 'notFound', message: u.pathname } });
        }),
      ),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  const row = (id: string) => host.querySelector(`[data-booking-id="${id}"]`);
  const actions = (id: string) =>
    [...(row(id)?.querySelectorAll('button[data-action]') ?? [])].map((b) => b.getAttribute('data-action')).sort();

  async function openCancelFor(id: string): Promise<void> {
    const btn = row(id)?.querySelector<HTMLButtonElement>('button[data-action="cancel"]');
    expect(btn, `${id} has a cancel button`).not.toBeNull();
    btn!.click();
    await flush();
  }

  function typeReason(value: string): void {
    const input = host.querySelector<HTMLInputElement>('input[name="reason"]');
    expect(input, 'cancel reason input').not.toBeNull();
    input!.value = value;
    input!.dispatchEvent(new Event('input', { bubbles: true }));
  }

  const confirm = () => host.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel"]')!;

  it('T1b-1 an IN_SERVICE row shows cancel and nothing else', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(row('bk-in_service'), 'IN_SERVICE row exists').not.toBeNull();
    expect(actions('bk-in_service')).toEqual(['cancel']);
  });

  it('T1b-4 a CHECKED_IN row shows cancel and keeps reissue QR, and nothing else', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(actions('bk-checked_in')).toEqual(['cancel', 'reissue-qr']);
  });

  it.each(['bk-in_service', 'bk-checked_in'])(
    'T1b-2 %s: cancel with a reason posts /api/bookings/{id}/cancel once with the trimmed reason',
    async (id) => {
      dispose = render(() => <DeskBookingsScreen />, host);
      await flush();
      await openCancelFor(id);
      typeReason('  customer left the counter  ');
      confirm().click();
      confirm()?.click();
      await flush();
      const cancels = posts.filter((p) => p.path === `/api/bookings/${id}/cancel`);
      expect(cancels).toHaveLength(1);
      expect(cancels[0].body?.reason).toBe('customer left the counter');
      expect(posts).toHaveLength(1);
    },
  );

  it('T1b-3 a blank reason is refused locally: nothing is posted and the reason-required text shows', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    await openCancelFor('bk-in_service');
    typeReason('   ');
    confirm().click();
    await flush();
    expect(posts).toHaveLength(0);
    expect(
      [...host.querySelectorAll('[role="alert"]')].some((el) => (el.textContent ?? '').includes(en.reasonRequired)),
    ).toBe(true);
  });
});
