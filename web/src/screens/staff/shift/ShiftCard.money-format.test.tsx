// #5127 (owner, 2026-10-26): the shift count and hand-over forms show money, not wire strings. A face value
// reads "1,000" / "0.25" (trailing zeros dropped, at least 2 places when a fraction remains), and the close
// count's expected amount reads "9,839.50". The wire string stays on each row as `data-face-value` (e2e readers
// parse it) and on `currency-expected` as `data-expected` (#5114 row 8). What is posted never changes.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ShiftCard } from './ShiftCard';
import { setLang } from '../../../shared/i18n';
import type { StaffCtx } from '../cards';
import {
  makeCountSubmitInput,
  makeDenominationRow,
  makeHandoverReceiverRow,
  makeMyShift,
  makeStaffFloorMyShift,
  makeStaffFloorView,
  makeStaffMeView,
  type CountSubmitInput,
} from '../../../shared/testing/fixtures';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000a1';
const OTHER = '00000000-0000-4000-8000-0000000000a2';
const SHIFT_ID = 'shift-open-001';

const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 13, currency: 'THB', faceValue: '0.2500', groupCode: 'COIN' }),
  makeDenominationRow({ id: 21, currency: 'USD', faceValue: '100.0000', groupCode: 'NOTE' }),
];

const MY_SHIFT = makeMyShift({
  id: SHIFT_ID,
  status: 'OPEN',
  openedAt: '2026-10-11T01:05:00Z',
  balances: [
    { code: 'THB', opening: '10000.0000', expected: '9839.5000', lowCashLevel: null, low: false },
    { code: 'USD', opening: '0.0000', expected: '5.0000', lowCashLevel: null, low: false },
  ],
});

function me() {
  return makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions: ['ShiftOperate'],
  });
}

function floor(open: boolean) {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: open ? makeStaffFloorMyShift({ id: SHIFT_ID, openedAt: '2026-10-11T01:05:00Z' }) : null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('#5127 shift forms show money: face values and the expected amount', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let posts: CountSubmitInput[];

  function stubViews() {
    const views: Record<string, () => Promise<Response>> = {
      cash_counts: async () => json(200, { version: 1, changed: true, data: [] }),
      handovers_pending: async () => json(200, { version: 1, changed: true, data: [] }),
      denominations: async () => json(200, { version: 1, changed: true, data: { rows: DENOMS } }),
      handover_receivers: async () =>
        json(200, {
          version: 1,
          changed: true,
          data: { rows: [makeHandoverReceiverRow({ userId: OTHER, name: 'Somchai' })] },
        }),
      my_shift: async () => json(200, { version: 1, changed: true, data: MY_SHIFT }),
    };
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = url.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && path.startsWith('/api/v3/views/')) {
          const handler = views[path.slice('/api/v3/views/'.length)];
          return handler ? handler() : json(404, { error: { code: 'notFound', message: 'not found' } });
        }
        if (method === 'POST' && path === '/api/cash-counts') {
          posts.push(makeCountSubmitInput(JSON.parse(typeof init?.body === 'string' ? init.body : '{}')));
          return json(200, { id: 'count-new-001' });
        }
        return json(200, {});
      }),
    );
  }

  async function settle() {
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
  }

  async function mount(open: boolean) {
    const ctx: StaffCtx = { me: me(), floor: floor(open), refresh: vi.fn(async () => {}) };
    dispose = render(() => <ShiftCard ctx={ctx} />, host);
    await settle();
  }

  const tid = (id: string) => `[data-testid="${id}"]`;
  const q = <T extends Element = HTMLElement>(sel: string) => host.querySelector<T>(sel);
  function must<T extends Element = HTMLElement>(sel: string): T {
    const el = q<T>(sel);
    expect(el, sel).not.toBeNull();
    return el!;
  }
  const qty = (id: number) => `input[data-denomination-id="${id}"]`;

  /** The row holding denomination `id`, and the face value it shows. */
  function row(id: number): HTMLElement {
    const r = must<HTMLInputElement>(qty(id)).closest<HTMLElement>('[data-denomination-row]');
    expect(r, `the row of denomination ${id}`).not.toBeNull();
    return r!;
  }
  const shown = (id: number) => row(id).querySelector('.xc-shift-denom-val')?.textContent?.trim();

  function type(input: HTMLInputElement, value: string) {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    posts = [];
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  it('row 2: the count form shows THB 1000.0000 as "1,000" and keeps the wire string as data-face-value', async () => {
    stubViews();
    await mount(false);
    must<HTMLButtonElement>(tid('shift-open-count')).click();
    await vi.waitFor(() => expect(q(qty(11))).not.toBeNull());

    expect(shown(11)).toBe('1,000');
    expect(row(11).getAttribute('data-face-value')).toBe('1000.0000');
  });

  it('row 3: the count form shows THB 0.2500 as "0.25"', async () => {
    stubViews();
    await mount(false);
    must<HTMLButtonElement>(tid('shift-open-count')).click();
    await vi.waitFor(() => expect(q(qty(13))).not.toBeNull());

    expect(shown(13)).toBe('0.25');
    expect(row(13).getAttribute('data-face-value')).toBe('0.2500');
  });

  it('row 4: the close count shows the expected amount as money, keeping data-expected as the wire string', async () => {
    stubViews();
    await mount(true);
    must<HTMLButtonElement>(tid('shift-close-count')).click();
    await vi.waitFor(() =>
      expect(q(`${tid('currency-group')}[data-currency="USD"]`), 'USD pre-added').not.toBeNull(),
    );

    const thb = must(`${tid('currency-group')}[data-currency="THB"] ${tid('currency-expected')}`);
    expect(thb.textContent).toContain('9,839.50');
    expect(thb.textContent, 'never the raw wire string').not.toContain('9839.5000');
    expect(thb.textContent, 'money keeps 2 places, not 4').not.toContain('9,839.5000');
    expect(thb.getAttribute('data-expected')).toBe('9839.5000');

    const usd = must(`${tid('currency-group')}[data-currency="USD"] ${tid('currency-expected')}`);
    expect(usd.textContent).toContain('5.00');
    expect(usd.textContent, 'money keeps 2 places, not 4').not.toContain('5.0000');
    expect(usd.getAttribute('data-expected')).toBe('5.0000');
  });

  it('row 5: the hand-over form shows face values as money too, with data-face-value', async () => {
    stubViews();
    await mount(true);
    must<HTMLButtonElement>(tid('shift-handover')).click();
    await vi.waitFor(() => expect(q(qty(21))).not.toBeNull());

    expect(shown(11)).toBe('1,000');
    expect(row(11).getAttribute('data-face-value')).toBe('1000.0000');
    expect(shown(13)).toBe('0.25');
    expect(shown(21)).toBe('100');
    expect(row(21).getAttribute('data-face-value')).toBe('100.0000');
  });

  it('row 6: formatting never reaches the API — a count posts denominationId and the typed quantity', async () => {
    stubViews();
    await mount(false);
    must<HTMLButtonElement>(tid('shift-open-count')).click();
    await vi.waitFor(() => expect(q(qty(11))).not.toBeNull());

    type(must<HTMLInputElement>(qty(11)), '10');
    must<HTMLButtonElement>(tid('shift-count-save')).click();
    await vi.waitFor(() => expect(posts.length).toBe(1));
    expect(posts[0]?.lines).toEqual([{ currency: 'THB', denominationId: 11, qty: '10' }]);
  });
});
