// #4909 (input-validation audit T9): the desk's new-booking dialog checks currency, phone and referral code
// before it sends, and a server refusal naming a field shows at that field (shared `fieldError`, #4897).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingCreateOutput,
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeDeskConfig,
  makeDeskRate,
  makeMe,
  makeMeRole,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';

interface Post {
  path: string;
  body: Record<string, unknown>;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const BRANCH_ID = 'XC-B01';
const SLOT_ID = 'slot-1';

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: BRANCH_ID })],
  branch: BRANCH_ID,
  branchName: BRANCH_ID,
  permissions: ['BookingsDesk', 'RatesRead'],
});

const DESK_BOOKINGS = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: 'bk-1',
      bookingNo: 'B261023-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Somchai Jaidee',
      phoneLast4: '5678',
      slotId: SLOT_ID,
      startAt: '2026-10-23T09:00:00Z',
      endAt: '2026-10-23T09:30:00Z',
    }),
  ],
});

// The currencies the desk config lists. Served under both desk config views, so the test does not
// depend on which one the screen reads.
const DESK_CONFIG = makeDeskConfig({
  branchId: BRANCH_ID,
  branchName: BRANCH_ID,
  rates: [makeDeskRate({ code: 'USD' }), makeDeskRate({ code: 'EUR' }), makeDeskRate({ code: 'JPY' })],
});

type CreateAnswer = { status: number; body: unknown };

function makeServer(createAnswer: () => CreateAnswer) {
  const posts: Post[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && u.pathname === '/api/v3/views/me') {
        return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
        return jsonResponse(200, { version: 1, changed: true, data: DESK_BOOKINGS });
      }
      if (
        method === 'GET' &&
        (u.pathname === '/api/v3/views/desk_config' || u.pathname === '/api/v3/views/staff_desk_config')
      ) {
        return jsonResponse(200, { version: 1, changed: true, data: DESK_CONFIG });
      }
      if (method === 'POST' && u.pathname === '/api/bookings') {
        posts.push({ path: u.pathname, body: JSON.parse(String(init?.body ?? '{}')) });
        const a = createAnswer();
        return jsonResponse(a.status, a.body);
      }
      return jsonResponse(404, { error: { code: 'notFound', message: u.pathname } });
    }),
  );
  return { posts };
}

const CREATED: CreateAnswer = {
  status: 200,
  body: makeBookingCreateOutput({
    id: 'bk-new',
    bookingNo: 'B261023-0099',
    status: 'CONFIRMED',
    sourceChannel: 'DESK',
    branchId: BRANCH_ID,
    slotId: SLOT_ID,
    customerId: 'cust-new',
    shopSide: 'SELL',
    currency: 'EUR',
    quantity: 100,
    qrToken: null,
  }),
};

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function setValue(el: HTMLInputElement | HTMLSelectElement, val: string): void {
  el.value = val;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('DeskBookingsScreen — new-booking dialog input checks (#4909)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('staff');
    setLang('en');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('staff');
    setLang('th');
  });

  async function openDialog(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await vi.waitFor(() => {
      expect(host.querySelector('[data-booking-id="bk-1"]')).not.toBeNull();
    });
    const btn = host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]');
    expect(btn, 'New booking button').not.toBeNull();
    btn!.click();
    await flush();
    expect(host.querySelector('.xc-modal'), 'new-booking dialog is open').not.toBeNull();
  }

  function input(name: string): HTMLInputElement {
    const el = host.querySelector<HTMLInputElement>(`.xc-modal input[name="${name}"]`);
    expect(el, `input[name="${name}"]`).not.toBeNull();
    return el!;
  }

  /** The worded line inside the form group of the named control, if any. */
  function lineAt(name: string): HTMLElement | null {
    const control = host.querySelector(`.xc-modal [name="${name}"]`);
    expect(control, `control [name="${name}"]`).not.toBeNull();
    const group = control!.closest('.xc-form-group');
    expect(group, `form group of ${name}`).not.toBeNull();
    return group!.querySelector<HTMLElement>('[role="alert"], .field-error');
  }

  function fillValid(): void {
    setValue(input('quantity'), '100');
    setValue(input('displayName'), 'Somchai');
    setValue(input('phone'), '0812345678');
  }

  function submit(): void {
    const btn = host.querySelector<HTMLButtonElement>('button[data-action="submit-booking"]');
    expect(btn, 'submit booking button').not.toBeNull();
    btn!.click();
  }

  it('N1: currency is a select of the currencies the desk config lists; there is no free-text currency input', async () => {
    makeServer(() => CREATED);
    await openDialog();

    expect(host.querySelector('.xc-modal input[name="currency"]'), 'no free-text currency input').toBeNull();
    const select = host.querySelector<HTMLSelectElement>('.xc-modal select[name="currency"]');
    expect(select, 'currency select').not.toBeNull();
    await vi.waitFor(() => {
      const codes = Array.from(select!.options).map((o) => o.value);
      expect(codes).toEqual(expect.arrayContaining(['USD', 'EUR', 'JPY']));
    });
  });

  it('N2: phone "12" (and blank) shows a worded line at the phone field and sends no request', async () => {
    const server = makeServer(() => CREATED);
    await openDialog();

    fillValid();
    setValue(input('phone'), '12');
    submit();
    await flush();

    expect(server.posts).toHaveLength(0);
    const line = lineAt('phone');
    expect(line, 'worded line at the phone field').not.toBeNull();
    expect(line!.textContent?.trim()).not.toBe('');

    setValue(input('phone'), '');
    submit();
    await flush();

    expect(server.posts).toHaveLength(0);
    expect(lineAt('phone'), 'worded line at a blank phone').not.toBeNull();
  });

  it('N3: referral code "ab" shows a worded line at the referral field and sends no request', async () => {
    const server = makeServer(() => CREATED);
    await openDialog();

    fillValid();
    setValue(input('referralCode'), 'ab');
    submit();
    await flush();

    expect(server.posts).toHaveLength(0);
    const line = lineAt('referralCode');
    expect(line, 'worded line at the referral field').not.toBeNull();
    expect(line!.textContent?.trim()).not.toBe('');
  });

  it('N4: a server 400 naming field "phone" shows its line at the phone field, not in the banner', async () => {
    const server = makeServer(() => ({
      status: 400,
      body: { error: { code: 'validationFailed', field: 'phone', message: 'ข้อมูลไม่ถูกต้อง' } },
    }));
    await openDialog();

    fillValid();
    submit();
    await vi.waitFor(() => expect(server.posts).toHaveLength(1));
    await flush();

    const line = lineAt('phone');
    expect(line, 'refusal shown at the phone field').not.toBeNull();
    const text = line!.textContent?.trim() ?? '';
    expect(text).not.toBe('');
    expect(text, 'en screen does not show the raw Thai server text').not.toContain('ข้อมูลไม่ถูกต้อง');
    expect(host.querySelector('.xc-modal .xc-error-banner'), 'no generic banner for a field refusal').toBeNull();
  });

  it('N5: all valid sends the request as today, with the chosen currency and the referral code', async () => {
    const server = makeServer(() => CREATED);
    await openDialog();

    const select = host.querySelector<HTMLSelectElement>('.xc-modal select[name="currency"]');
    expect(select, 'currency select').not.toBeNull();
    await vi.waitFor(() => {
      expect(Array.from(select!.options).map((o) => o.value)).toContain('EUR');
    });
    setValue(select!, 'EUR');
    fillValid();
    setValue(input('referralCode'), ' abc123 ');
    submit();
    await vi.waitFor(() => expect(server.posts).toHaveLength(1));

    expect(server.posts[0]!.body).toEqual({
      branch: BRANCH_ID,
      branchId: BRANCH_ID,
      slotId: SLOT_ID,
      shopSide: 'SELL',
      currency: 'EUR',
      quantity: 100,
      displayName: 'Somchai',
      phone: '0812345678',
      referralCode: 'abc123',
    });
    await flush();
    expect(host.querySelector('.xc-modal'), 'dialog closes on success').toBeNull();
  });
});
