// Cash forms reuse CurrencyCount (owner screenshot 2026-11-04, xchange#564): the handover form starts with only
// the currencies the teller's drawer holds, and the teller adds any other currency through the search box.
//
// Pre-added (locked, no remove button), the same rule as the CLOSE count (CountForm.tsx `countLocked`):
//   THB, plus every `my_shift` balance whose `expected` is numerically non-zero, in balance order.
// A currency that is not on the form sends no lines, as a blank field does today. The request body is unchanged.
//
// DOM contract (#5114 §4): rows keep `[data-denomination-row]` / `input[data-denomination-id]` and
// `[data-field-error="lines.<id>.qty"]`; CurrencyCount adds `currency-search`, `currency-option[data-currency]`,
// `currency-group[data-currency]`, `currency-remove`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ShiftCard } from './ShiftCard';
import { setLang } from '../../../shared/i18n';
import type { StaffCtx } from '../cards';
import {
  makeDenominationRow,
  makeHandoverReceiverRow,
  makeHandoverStartInput,
  makeMyShift,
  makeStaffFloorMyShift,
  makeStaffFloorView,
  makeStaffMeView,
  type HandoverStartInput,
} from '../../../shared/testing/fixtures';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000a1';
const OTHER = '00000000-0000-4000-8000-0000000000a2';
const SHIFT_ID = 'shift-open-001';

const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '500.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 21, currency: 'USD', faceValue: '50.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 22, currency: 'USD', faceValue: '100.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 31, currency: 'EUR', faceValue: '20.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 41, currency: 'JPY', faceValue: '1000.0000', groupCode: 'NOTE' }),
];

// USD expected non-zero ⇒ locked; EUR expected zero ⇒ not pre-added; JPY has no balance ⇒ not pre-added.
const MY_SHIFT = makeMyShift({
  id: SHIFT_ID,
  status: 'OPEN',
  openedAt: '2026-10-11T01:05:00Z',
  balances: [
    { code: 'THB', opening: '10000.0000', expected: '9839.5000', lowCashLevel: null, low: false },
    { code: 'USD', opening: '0.0000', expected: '150.0000', lowCashLevel: null, low: false },
    { code: 'EUR', opening: '5.0000', expected: '0.0000', lowCashLevel: null, low: false },
  ],
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

describe('cash forms reuse CurrencyCount: the handover form', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let posts: HandoverStartInput[];
  let handoverStatus: number;

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
        if (method === 'POST' && path === '/api/handovers') {
          posts.push(makeHandoverStartInput(JSON.parse(typeof init?.body === 'string' ? init.body : '{}')));
          if (handoverStatus !== 200) {
            return json(handoverStatus, {
              error: { code: 'validation', message: 'qty too large', field: 'lines[0].qty' },
            });
          }
          return json(200, { id: 'handover-new-001' });
        }
        return json(200, {});
      }),
    );
  }

  async function settle() {
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
  }

  async function mount() {
    const me = makeStaffMeView({
      userId: ME,
      name: 'Ploy',
      roles: [],
      branchId: BRANCH,
      branchName: 'XC-B',
      permissions: ['ShiftOperate'],
    });
    const floor = makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: '2026-10-11T01:00:00Z',
      myShift: makeStaffFloorMyShift({ id: SHIFT_ID, openedAt: '2026-10-11T01:05:00Z' }),
      cutoff: null,
      target: null,
      safe: null,
    });
    const ctx: StaffCtx = { me, floor, refresh: vi.fn(async () => {}) };
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
  const group = (code: string) => `${tid('currency-group')}[data-currency="${code}"]`;
  const option = (code: string) => `${tid('currency-option')}[data-currency="${code}"]`;
  const qty = (id: number) => `input[data-denomination-id="${id}"]`;

  function type(input: HTMLInputElement, value: string) {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  async function openHandover() {
    must<HTMLButtonElement>(tid('shift-handover')).click();
    await vi.waitFor(() => {
      expect(q(qty(11)), 'THB inputs drawn').not.toBeNull();
      expect(q(group('USD')), 'USD pre-added from my_shift').not.toBeNull();
      expect(q(`${tid('shift-receiver')} option[value="${OTHER}"]`), 'receiver loaded').not.toBeNull();
    });
    return must<HTMLFormElement>(tid('shift-handover-form'));
  }

  async function addBySearch(code: string) {
    type(must<HTMLInputElement>(tid('currency-search')), code.toLowerCase());
    await vi.waitFor(() => expect(q(option(code)), option(code)).not.toBeNull());
    must<HTMLButtonElement>(option(code)).click();
    await vi.waitFor(() => expect(q(group(code)), group(code)).not.toBeNull());
  }

  function pickReceiver() {
    const sel = must<HTMLSelectElement>(tid('shift-receiver'));
    sel.value = OTHER;
    sel.dispatchEvent(new Event('input', { bubbles: true }));
    sel.dispatchEvent(new Event('change', { bubbles: true }));
  }

  async function save() {
    must<HTMLButtonElement>(tid('shift-handover-save')).click();
    await vi.waitFor(() => expect(posts.length).toBe(1));
  }

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    posts = [];
    handoverStatus = 200;
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  it('H1: the handover form pre-adds THB and every non-zero my_shift balance, locked, plus a search box', async () => {
    stubViews();
    await mount();
    const form = await openHandover();

    const groups = Array.from(form.querySelectorAll(tid('currency-group'))).map((g) => g.getAttribute('data-currency'));
    expect(groups).toEqual(['THB', 'USD']);
    expect(q(qty(31)), 'EUR (expected 0) not on the form until added').toBeNull();
    expect(q(qty(41)), 'JPY (no balance) not on the form until added').toBeNull();
    must<HTMLInputElement>(tid('currency-search'));
    expect(must(group('THB')).querySelector(tid('currency-remove')), 'THB is locked').toBeNull();
    expect(must(group('USD')).querySelector(tid('currency-remove')), 'USD is locked').toBeNull();
  });

  it('H2: searching "eur" offers EUR; picking it adds its notes with a remove button', async () => {
    stubViews();
    await mount();
    await openHandover();

    await addBySearch('EUR');
    expect(q(qty(31))).not.toBeNull();
    expect(must(group('EUR')).querySelector(tid('currency-remove')), 'an added currency can be removed').not.toBeNull();
    expect(must<HTMLInputElement>(tid('currency-search')).value).toBe('');
  });

  it('H3: the request body lists exactly the typed notes, as before', async () => {
    stubViews();
    await mount();
    await openHandover();
    await addBySearch('EUR');
    pickReceiver();

    type(must<HTMLInputElement>(qty(11)), '9');
    type(must<HTMLInputElement>(qty(22)), '1');
    type(must<HTMLInputElement>(qty(31)), '2');
    await save();

    expect(posts[0]!.fromShiftId).toBe(SHIFT_ID);
    expect(posts[0]!.receiverId).toBe(OTHER);
    expect(posts[0]!.lines).toEqual([
      { currency: 'THB', denominationId: 11, qty: '9' },
      { currency: 'USD', denominationId: 22, qty: '1' },
      { currency: 'EUR', denominationId: 31, qty: '2' },
    ]);
  });

  it('H4: removing an added currency clears its notes and sends no line for it', async () => {
    stubViews();
    await mount();
    await openHandover();
    await addBySearch('EUR');
    pickReceiver();

    type(must<HTMLInputElement>(qty(31)), '2');
    must<HTMLButtonElement>(`${group('EUR')} ${tid('currency-remove')}`).click();
    await vi.waitFor(() => expect(q(group('EUR'))).toBeNull());
    type(must<HTMLInputElement>(qty(12)), '3');
    await save();

    expect(posts[0]!.lines).toEqual([{ currency: 'THB', denominationId: 12, qty: '3' }]);
  });

  it('H5: a server field error on a line lands on that note row', async () => {
    stubViews();
    handoverStatus = 422;
    await mount();
    await openHandover();
    pickReceiver();

    type(must<HTMLInputElement>(qty(21)), '999999');
    await save();

    await vi.waitFor(() => expect(q('[data-field-error="lines.21.qty"]'), 'error on USD 50').not.toBeNull());
    const row = must(qty(21)).closest('[data-denomination-row]');
    expect(row?.querySelector('[data-field-error="lines.21.qty"]')).not.toBeNull();
  });

  it('H6: English shows the same groups and search box', async () => {
    setLang('en');
    stubViews();
    await mount();
    const form = await openHandover();

    const groups = Array.from(form.querySelectorAll(tid('currency-group'))).map((g) => g.getAttribute('data-currency'));
    expect(groups).toEqual(['THB', 'USD']);
    await addBySearch('JPY');
    expect(q(qty(41))).not.toBeNull();
  });
});
