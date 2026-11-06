// #5114 (owner, 2026-10-25/26): the shift count form starts with only the currencies the teller holds, and the
// teller adds any other currency by typing it into a search box, then fills in its notes. Opening and closing
// work the same way.
//
// Pre-added (locked, no remove button):
//   OPEN   THB only (there is no shift yet, so nothing else is known for the teller);
//   CLOSE  THB, plus every `my_shift` balance whose `expected` is numerically non-zero, in balance order.
// A currency that is not on the form sends no lines (= 0), as a blank field does today.
// The handover form is not part of this ticket: it keeps listing every denomination.
//
// DOM contract (#5114 §4): rows keep `[data-denomination-row]` / `input[data-denomination-id]`; new hooks are
// `currency-search`, `currency-option[data-currency]`, `currency-notfound`, `currency-group[data-currency]`,
// `currency-remove`, `currency-expected[data-expected]`.
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
  makeOpenShiftChoice,
  makeOpenShiftChoices,
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
];

const MY_SHIFT_USD = makeMyShift({
  id: SHIFT_ID,
  status: 'OPEN',
  openedAt: '2026-10-11T01:05:00Z',
  balances: [
    { code: 'THB', opening: '10000.0000', expected: '9839.5000', lowCashLevel: null, low: false },
    { code: 'USD', opening: '0.0000', expected: '1.0000', lowCashLevel: null, low: false },
    { code: 'EUR', opening: '5.0000', expected: '0.0000', lowCashLevel: null, low: false },
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

describe('#5114 shift count form: search and add currencies, pre-add what the shift holds', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let posts: CountSubmitInput[];

  // CS-15m: the OPEN count form and the handover accept now read open_shift_choices (ShiftPicker); one covering
  // shift keeps Save and Accept usable as before. Stub restated per the lane controller's ruling on PR 6274.
  const ONE_SHIFT = makeOpenShiftChoices({ choices: [makeOpenShiftChoice({ templateId: 'tpl-1' })], preselect: 'tpl-1' });

  function stubViews(overrides: Record<string, () => Promise<Response>> = {}) {
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
      my_shift: async () => json(200, { version: 1, changed: true, data: MY_SHIFT_USD }),
      open_shift_choices: async () => json(200, { version: 1, changed: true, data: ONE_SHIFT }),
      ...overrides,
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
  const group = (code: string) => `${tid('currency-group')}[data-currency="${code}"]`;
  const option = (code: string) => `${tid('currency-option')}[data-currency="${code}"]`;
  const qty = (id: number) => `input[data-denomination-id="${id}"]`;

  function type(input: HTMLInputElement, value: string) {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  async function openForm(control: 'shift-open-count' | 'shift-close-count') {
    must<HTMLButtonElement>(tid(control)).click();
    await vi.waitFor(() => expect(q(qty(11)), 'THB inputs drawn').not.toBeNull());
    return must<HTMLFormElement>(tid('shift-count-form'));
  }

  async function addBySearch(code: string) {
    type(must<HTMLInputElement>(tid('currency-search')), code.toLowerCase());
    await vi.waitFor(() => expect(q(option(code)), option(code)).not.toBeNull());
    must<HTMLButtonElement>(option(code)).click();
    await vi.waitFor(() => expect(q(group(code)), group(code)).not.toBeNull());
  }

  async function save() {
    // CS-15m: Save now waits for open_shift_choices to answer (ShiftPicker), so wait for it to be enabled before clicking.
    await vi.waitFor(() => expect(must<HTMLButtonElement>(tid('shift-count-save')).disabled).toBe(false));
    must<HTMLButtonElement>(tid('shift-count-save')).click();
    await vi.waitFor(() => expect(posts.length).toBe(1));
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

  it('row 1: the open count form shows only THB, with a search box and no way to remove THB', async () => {
    stubViews();
    await mount(false);
    const form = await openForm('shift-open-count');

    expect(form.querySelectorAll(tid('currency-group')).length).toBe(1);
    must(group('THB'));
    expect(q(qty(11))).not.toBeNull();
    expect(q(qty(12))).not.toBeNull();
    expect(q(qty(21)), 'USD not on the form until added').toBeNull();
    expect(q(qty(31)), 'EUR not on the form until added').toBeNull();
    must<HTMLInputElement>(tid('currency-search'));
    expect(must(group('THB')).querySelector(tid('currency-remove')), 'THB is locked').toBeNull();
  });

  it('row 2: typing "usd" offers USD; picking it adds its notes and clears the search', async () => {
    stubViews();
    await mount(false);
    await openForm('shift-open-count');

    await addBySearch('USD');

    const usd = must(group('USD'));
    expect(usd.querySelector(qty(21))).not.toBeNull();
    expect(usd.querySelector(qty(22))).not.toBeNull();
    expect(must<HTMLInputElement>(tid('currency-search')).value).toBe('');
    expect(host.querySelectorAll(tid('currency-option')).length).toBe(0);
    expect(usd.querySelector(tid('currency-remove')), 'an added currency can be removed').not.toBeNull();
  });

  it('row 3: a currency is also found by its name in the screen language (th: "ดอลลาร์")', async () => {
    stubViews();
    await mount(false);
    await openForm('shift-open-count');

    type(must<HTMLInputElement>(tid('currency-search')), 'ดอลลาร์');
    await vi.waitFor(() => expect(q(option('USD'))).not.toBeNull());
    expect(q(option('EUR')), 'EUR is not a dollar').toBeNull();
  });

  it('row 4: a currency already on the form, or one with no notes, is not offered and the not-found line shows', async () => {
    stubViews();
    await mount(false);
    await openForm('shift-open-count');
    const search = must<HTMLInputElement>(tid('currency-search'));

    type(search, 'THB');
    await vi.waitFor(() => expect(q(tid('currency-notfound'))).not.toBeNull());
    expect(host.querySelectorAll(tid('currency-option')).length).toBe(0);

    type(search, 'XYZ');
    await vi.waitFor(() => expect(q(tid('currency-notfound'))).not.toBeNull());
    expect(host.querySelectorAll(tid('currency-option')).length).toBe(0);

    type(search, '');
    await settle();
    expect(q(tid('currency-notfound')), 'an empty search shows no not-found line').toBeNull();
    expect(host.querySelectorAll(tid('currency-option')).length, 'an empty search lists nothing').toBe(0);
  });

  it('row 5: Enter in the search box adds the first match and does not submit the count', async () => {
    stubViews();
    await mount(false);
    await openForm('shift-open-count');
    const search = must<HTMLInputElement>(tid('currency-search'));

    type(search, 'usd');
    await vi.waitFor(() => expect(q(option('USD'))).not.toBeNull());
    const enter = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    search.dispatchEvent(enter);
    // jsdom never submits a form on a synthetic Enter, so the browser's implicit submission is caught here.
    expect(enter.defaultPrevented, 'Enter in the search box must not submit the count form').toBe(true);
    await vi.waitFor(() => expect(q(group('USD'))).not.toBeNull());
    await settle();
    expect(posts, 'Enter in the search box never posts the count').toEqual([]);
  });

  it('row 6: removing an added currency drops its lines, and adding it back shows empty notes', async () => {
    stubViews();
    await mount(false);
    await openForm('shift-open-count');

    await addBySearch('USD');
    type(must<HTMLInputElement>(qty(21)), '5');
    must<HTMLButtonElement>(`${group('USD')} ${tid('currency-remove')}`).click();
    await vi.waitFor(() => expect(q(group('USD'))).toBeNull());
    expect(q(qty(21))).toBeNull();

    await addBySearch('USD');
    expect(must<HTMLInputElement>(qty(21)).value, 'a re-added currency starts empty').toBe('');

    must<HTMLButtonElement>(`${group('USD')} ${tid('currency-remove')}`).click();
    await vi.waitFor(() => expect(q(group('USD'))).toBeNull());
    type(must<HTMLInputElement>(qty(11)), '2');
    await save();
    expect(posts[0]?.lines).toEqual([{ currency: 'THB', denominationId: 11, qty: '2' }]);
  });

  it('row 7: an open count posts kind OPEN with exactly the typed lines of THB and the added USD', async () => {
    stubViews();
    await mount(false);
    await openForm('shift-open-count');

    await addBySearch('USD');
    type(must<HTMLInputElement>(qty(11)), '9');
    type(must<HTMLInputElement>(qty(22)), '3');
    await save();

    expect(posts[0]?.kind).toBe('OPEN');
    expect([...(posts[0]?.lines ?? [])].sort((a, b) => a.denominationId - b.denominationId)).toEqual([
      { currency: 'THB', denominationId: 11, qty: '9' },
      { currency: 'USD', denominationId: 22, qty: '3' },
    ]);
  });

  it('row 8: the close count pre-adds THB and every currency the shift is expected to hold, locked, with its expected amount', async () => {
    stubViews();
    await mount(true);
    await openForm('shift-close-count');

    await vi.waitFor(() => expect(q(group('USD')), 'USD pre-added from my_shift').not.toBeNull());
    must(group('THB'));
    expect(q(group('EUR')), 'EUR expected 0.0000 is not pre-added').toBeNull();
    expect(q(qty(31))).toBeNull();

    const usd = must(group('USD'));
    expect(usd.querySelector(tid('currency-remove')), 'a held currency cannot be removed').toBeNull();
    expect(must(group('THB')).querySelector(tid('currency-remove'))).toBeNull();
    expect(usd.querySelector(tid('currency-expected'))?.getAttribute('data-expected')).toBe('1.0000');
    expect(must(group('THB')).querySelector(tid('currency-expected'))?.getAttribute('data-expected')).toBe(
      '9839.5000',
    );

    const codes = [...host.querySelectorAll(tid('currency-group'))].map((g) => g.getAttribute('data-currency'));
    expect(codes, 'THB first, then the balances in order').toEqual(['THB', 'USD']);
  });

  it('row 9: when my_shift cannot be read, the close count still shows THB and saves', async () => {
    stubViews({
      my_shift: async () => json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }),
    });
    await mount(true);
    await openForm('shift-close-count');
    await settle();

    must(group('THB'));
    expect(q(group('USD'))).toBeNull();
    type(must<HTMLInputElement>(qty(12)), '4');
    await save();
    expect(posts[0]?.kind).toBe('CLOSE');
    expect(posts[0]?.lines).toEqual([{ currency: 'THB', denominationId: 12, qty: '4' }]);
  });

  it('row 10: the handover form is unchanged and lists every denomination with no search step', async () => {
    stubViews();
    await mount(true);

    must<HTMLButtonElement>(tid('shift-handover')).click();
    await vi.waitFor(() => expect(q(qty(31))).not.toBeNull());
    for (const d of DENOMS) expect(q(qty(d.id)), `handover input ${d.id}`).not.toBeNull();
    expect(q(tid('currency-search')), 'no search box on the handover form').toBeNull();
  });
});
