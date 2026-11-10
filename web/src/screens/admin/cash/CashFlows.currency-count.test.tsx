// Cash forms reuse CurrencyCount (owner screenshot 2026-11-04, xchange#564): the admin #cash
// "สร้างการโอนย้ายเงินสด" form lists only THB at first, and the user adds any other currency through the
// search box, as the staff shift count does (#5114). No transfer kind implies another currency.
//
// Pre-added (locked, no remove button): THB. A currency that is not on the form sends no lines, as a blank field
// does today. The request body is unchanged.
//
// DOM contract (#5114 §4): rows are `[data-denomination-row]` with `input[data-denomination-id]` and
// `[data-field-error="lines.<id>.qty"]`; CurrencyCount adds `currency-search`, `currency-option[data-currency]`,
// `currency-group[data-currency]`, `currency-remove`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeDenominationRow, makeTransferBranchRow } from '../../../shared/testing/fixtures';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { CashFlows } from './CashFlows';

const VIEWS = '/api/v3/views/';

const DENOMS = [
  makeDenominationRow({ id: 1, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 2, currency: 'THB', faceValue: '100.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 3, currency: 'USD', faceValue: '100.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 4, currency: 'EUR', faceValue: '20.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 5, currency: 'CNY', faceValue: '100.0000', groupCode: 'NOTE' }),
];

describe('cash forms reuse CurrencyCount: admin #cash new-transfer form', () => {
  let root: HTMLElement | undefined;
  let dispose: (() => void) | undefined;
  let created: { kind: string; lines: unknown }[];

  beforeEach(() => {
    setLang('th');
    created = [];
    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const view = url.pathname.startsWith(VIEWS) ? url.pathname.slice(VIEWS.length) : url.pathname;
      const json = (body: unknown, status = 200) =>
        new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

      if (method === 'POST' && url.pathname === '/api/cash-transfers') {
        created.push(JSON.parse(typeof init?.body === 'string' ? init.body : '{}'));
        return json({ id: 'tr-new', status: 'DRAFT' });
      }
      if (view === 'me') {
        return json({
          version: 1,
          changed: true,
          data: {
            id: 'u-admin',
            name: 'Admin',
            roles: ['ADMIN'],
            permissions: ['ShiftCloseCertify', 'CashManage', 'FinanceReconcile'],
            branch: 'A',
            branchName: 'Branch A',
            branches: [{ id: 'A', code: 'A', name: 'Branch A' }],
          },
        });
      }
      if (view === 'cash_counts') return json({ version: 1, changed: true, data: [] });
      if (view === 'cash_transfers') return json({ version: 1, changed: true, data: [] });
      if (view === 'denominations') return json({ version: 1, changed: true, data: { rows: DENOMS } });
      if (view === 'transfer_branches') {
        return json({ version: 1, changed: true, data: { rows: [makeTransferBranchRow({ branchId: 'X', name: 'Branch X' })] } });
      }
      return json({ version: 1, changed: true, data: undefined });
    });
  });

  afterEach(async () => {
    await pendingWrite.abandon();
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  const tid = (id: string) => `[data-testid="${id}"]`;
  const group = (code: string) => `${tid('currency-group')}[data-currency="${code}"]`;
  const option = (code: string) => `${tid('currency-option')}[data-currency="${code}"]`;
  const qty = (id: number) => `input[data-denomination-id="${id}"]`;

  function form(): HTMLFormElement {
    const f = root?.querySelector<HTMLFormElement>('[data-section="new-transfer"] form');
    expect(f, 'the new-transfer form').toBeTruthy();
    return f!;
  }
  function must<T extends Element = HTMLElement>(sel: string): T {
    const el = form().querySelector<T>(sel);
    expect(el, sel).not.toBeNull();
    return el!;
  }
  const groups = () =>
    Array.from(form().querySelectorAll(tid('currency-group'))).map((g) => g.getAttribute('data-currency'));

  function type(el: HTMLInputElement | HTMLSelectElement, value: string) {
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  async function mount() {
    const [b] = createSignal('A');
    const [shown] = createSignal(true);
    const [shifts] = createSignal([]);
    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(
      () => (
        <>
          <CashFlows branch={b} shown={shown} shifts={shifts} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    await vi.waitFor(() => expect(form().querySelector(qty(1)), 'THB inputs drawn').not.toBeNull());
    type(must<HTMLSelectElement>('#transfer-kind'), 'EXTERNAL_IN');
  }

  async function addBySearch(code: string) {
    type(must<HTMLInputElement>(tid('currency-search')), code.toLowerCase());
    await vi.waitFor(() => expect(form().querySelector(option(code)), option(code)).not.toBeNull());
    must<HTMLButtonElement>(option(code)).click();
    await vi.waitFor(() => expect(form().querySelector(group(code)), group(code)).not.toBeNull());
  }

  async function create() {
    must<HTMLButtonElement>('button[data-action="create"]').click();
    await vi.waitFor(() => expect(created.length).toBe(1));
  }

  it('A1: the form shows THB only, locked, with a search box; no other currency is listed', async () => {
    await mount();

    expect(groups()).toEqual(['THB']);
    for (const id of [3, 4, 5]) expect(form().querySelector(qty(id)), `denomination ${id} not listed`).toBeNull();
    must<HTMLInputElement>(tid('currency-search'));
    expect(must(group('THB')).querySelector(tid('currency-remove')), 'THB is locked').toBeNull();
  });

  it('A2: typing "usd" offers USD only; picking it adds its notes and clears the search', async () => {
    await mount();
    type(must<HTMLInputElement>(tid('currency-search')), 'usd');
    await vi.waitFor(() => expect(form().querySelector(option('USD'))).not.toBeNull());
    expect(form().querySelectorAll(tid('currency-option'))).toHaveLength(1);

    must<HTMLButtonElement>(option('USD')).click();
    await vi.waitFor(() => expect(form().querySelector(qty(3))).not.toBeNull());
    expect(groups()).toEqual(['THB', 'USD']);
    expect(must<HTMLInputElement>(tid('currency-search')).value).toBe('');
  });

  it('A3: the create body lists exactly the typed notes, as before', async () => {
    await mount();
    await addBySearch('USD');

    type(must<HTMLInputElement>(qty(1)), '2');
    type(must<HTMLInputElement>(qty(3)), '5');
    await create();

    expect(created[0]!.kind).toBe('EXTERNAL_IN');
    expect(created[0]!.lines).toEqual([
      { currency: 'THB', denominationId: 1, qty: '2' },
      { currency: 'USD', denominationId: 3, qty: '5' },
    ]);
  });

  it('A4: removing an added currency clears its notes and sends no line for it', async () => {
    await mount();
    await addBySearch('EUR');

    type(must<HTMLInputElement>(qty(4)), '6');
    must<HTMLButtonElement>(`${group('EUR')} ${tid('currency-remove')}`).click();
    await vi.waitFor(() => expect(form().querySelector(group('EUR'))).toBeNull());
    type(must<HTMLInputElement>(qty(2)), '1');
    await create();

    expect(created[0]!.lines).toEqual([{ currency: 'THB', denominationId: 2, qty: '1' }]);
  });

  it('A5: a quantity of 0 is refused on that note row and nothing is sent', async () => {
    await mount();
    await addBySearch('CNY');

    type(must<HTMLInputElement>(qty(5)), '0');
    must<HTMLButtonElement>('button[data-action="create"]').click();
    await new Promise((r) => setTimeout(r, 0));

    expect(created).toHaveLength(0);
    const row = must(qty(5)).closest('[data-denomination-row]');
    expect(row?.querySelector('[data-field-error="lines.5.qty"]'), 'error on CNY 100').not.toBeNull();
  });

  it('A6: English shows the same groups and search box', async () => {
    setLang('en');
    await mount();
    expect(groups()).toEqual(['THB']);
    await addBySearch('EUR');
    expect(form().querySelector(qty(4))).not.toBeNull();
  });
});
