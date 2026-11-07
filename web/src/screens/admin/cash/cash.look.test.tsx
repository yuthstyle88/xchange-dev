// OL-6 wave B admin-cash (audit #564 issuecomment-6702970101, shot 020): the cash page's look.
// - The new-transfer form drew one quantity box per denomination row of the server (267 on xc-v3) before anything was
//   chosen, so the page ran ~7000 px. A currency picker now shows only that currency's boxes; a box already typed in,
//   or one the server refused, stays shown so the whole transfer is visible. Every box stays in the DOM (hidden), so the
//   CA-9 handles (`input[data-denomination]`, Cash.ca9.test.tsx) keep working.
// - A long transfer note, creator line or movement reason widened its table past the card (1559 px in a 914 px card);
//   those cells now wrap (`.cell-wrap`).
// - The stock footnote sat loose on the grey page between two cards; it now sits inside the stock card.
// Written before the fix by claude/bbe9ce8b for the OL-6 sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import {
  makeAdminCashMovementRow,
  makeBranchRef,
  makeCashTransferLineRow,
  makeCashTransferRow,
  makeDenominationRow,
  makeMe,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { menu } from './menu';
import { en, th } from './i18n';

const DENOMINATIONS = [
  makeDenominationRow({ id: 11, currency: 'USD', faceValue: '100', groupCode: 'G' }),
  makeDenominationRow({ id: 12, currency: 'USD', faceValue: '50', groupCode: 'G' }),
  makeDenominationRow({ id: 21, currency: 'EUR', faceValue: '20', groupCode: 'G' }),
  makeDenominationRow({ id: 31, currency: 'THB', faceValue: '1000', groupCode: 'G' }),
];

const LONG_NOTE = 'assumed top-up per ORDER chat 20:47-20:57 (ASSUMPTION: THB 150,000, smallest round amount)';

const TRANSFERS = [
  makeCashTransferRow({
    id: 'tr-1',
    kind: 'EXTERNAL_IN',
    status: 'RECEIVED',
    fromBranchId: 'MAIN',
    toBranchId: null,
    fromShiftId: null,
    toShiftId: null,
    createdBy: 'EMP-002',
    createdByName: 'Finance',
    createdAt: '2026-11-06T03:58:15Z',
    note: LONG_NOTE,
    lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 31, qtySent: '150', qtyReceived: '150' })],
  }),
];

const MOVE = makeAdminCashMovementRow({
  id: 'MV-1',
  created: '2026-10-06T03:00:00.000Z',
  shiftId: 'S-0001',
  direction: 'OUT',
  currency: 'USD',
  amount: '10.25',
  reason: LONG_NOTE,
});

const ADMIN_CASH = {
  openShifts: [{ shiftId: 'S-0001', tellerId: 'u-1', tellerName: 'EMP-007', openedAt: '2026-10-06T02:30:00.000Z' }],
  stock: [{ code: 'THB', opening: '100.00', net: '0.00', expected: '100.00' }],
  stats: { opening: '100.00', incoming: '0.00', outgoing: '0.00', remaining: '100.00' },
  reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
  currencies: ['THB', 'USD'],
  lastCounted: null,
};

interface Sent {
  method: string;
  path: string;
  body: string | null;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

class Server {
  sent: Sent[] = [];
  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    this.sent.push({ method, path: u.pathname, body: typeof init?.body === 'string' ? init.body : null });
    const ok = (data: unknown) => json(200, { version: 1, changed: true, data });
    if (method !== 'GET') return json(200, { id: 'x', status: 'OK' });
    switch (u.pathname) {
      case '/api/v3/views/me':
        return ok(
          makeMe({
            id: 'u-admin',
            name: 'ผู้ดูแลทดสอบ',
            roles: [],
            permissions: ['ShiftCloseCertify', 'CashManage', 'FinanceReconcile'],
            branch: 'MAIN',
            branchName: 'สำนักงานใหญ่',
            branches: [makeBranchRef({ id: 'MAIN', code: 'MAIN', name: 'สำนักงานใหญ่' })],
          }),
        );
      case QUEUE_CARD_PATH:
        return json(200, queueCardUnchanged());
      case '/api/v3/views/admin_branches':
        return ok({ branches: [{ id: 'MAIN', name: 'สำนักงานใหญ่' }] });
      case '/api/v3/views/admin_cash':
        return ok(ADMIN_CASH);
      case '/api/v3/views/admin_cash_movements':
        return ok([MOVE]);
      case '/api/v3/views/admin_shifts':
        return ok([]);
      case '/api/v3/views/cash_counts':
        return ok([]);
      case '/api/v3/views/cash_transfers':
        return ok(TRANSFERS);
      case '/api/v3/views/denominations':
        return ok({ rows: DENOMINATIONS });
      case '/api/v3/views/transfer_branches':
        return ok({ rows: [] });
      default:
        return ok(null);
    }
  };
}

let server: Server;
let dispose: (() => void) | undefined;
let root: HTMLElement;

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount(): Promise<HTMLElement> {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await settle();
  return root;
}

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  vi.stubGlobal('matchMedia', (query: string) => ({
    media: query,
    matches: false,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    onchange: null,
    dispatchEvent: () => false,
  }));
  location.hash = '#cash';
  setLang('th');
});

afterEach(async () => {
  await settle();
  await pendingWrite.abandon();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
});

const form = (el: ParentNode) => el.querySelector<HTMLElement>('[data-section="new-transfer"]')!;
const picker = (el: ParentNode) => form(el).querySelector<HTMLSelectElement>('select[name="lineCurrency"]');
const box = (el: ParentNode, id: number) => form(el).querySelector<HTMLInputElement>(`input[data-denomination="${id}"]`)!;
/** The box's own wrapper inside `.transfer-lines`: the element that is hidden or shown. */
const line = (el: ParentNode, id: number) => {
  let n: HTMLElement = box(el, id);
  while (n.parentElement && !n.parentElement.classList.contains('transfer-lines')) n = n.parentElement;
  return n;
};
const shownIds = (el: ParentNode) =>
  DENOMINATIONS.map((d) => d.id).filter((id) => !line(el, id).hidden && line(el, id).closest('[hidden]') === null);

function choose(select: HTMLSelectElement, value: string): void {
  select.value = value;
  select.dispatchEvent(new Event('input', { bubbles: true }));
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('admin-cash: the new-transfer form asks the currency first', () => {
  it('CL-1 a currency picker lists each currency of the denominations once, in the server order, after a blank choice', async () => {
    const el = await mount();
    const p = picker(el);
    expect(p, 'select[name="lineCurrency"] in the new-transfer form').not.toBeNull();
    expect([...p!.options].map((o) => o.value)).toEqual(['', 'USD', 'EUR', 'THB']);
    expect(p!.value).toBe('');
  });

  it('CL-2 before a currency is chosen no quantity box shows, though every box stays in the form', async () => {
    const el = await mount();
    expect(form(el).querySelectorAll('input[data-denomination]')).toHaveLength(4);
    expect(shownIds(el)).toEqual([]);
  });

  it("CL-3 choosing USD shows USD's boxes only", async () => {
    const el = await mount();
    choose(picker(el)!, 'USD');
    await settle();
    expect(shownIds(el)).toEqual([11, 12]);
    choose(picker(el)!, 'EUR');
    await settle();
    expect(shownIds(el)).toEqual([21]);
  });

  it('CL-4 a typed box stays shown and keeps its value after switching currency, and create sends every typed line', async () => {
    const el = await mount();
    choose(form(el).querySelector<HTMLSelectElement>('select[name="kind"]')!, 'EXTERNAL_OUT');
    choose(picker(el)!, 'EUR');
    await settle();
    type(box(el, 21), '10');
    choose(picker(el)!, 'USD');
    await settle();
    type(box(el, 12), '4');
    await settle();
    expect(shownIds(el)).toEqual([11, 12, 21]);
    expect(box(el, 21).value).toBe('10');
    form(el).querySelector<HTMLButtonElement>('button[data-action="create"]')!.click();
    await settle();
    const w = server.sent.filter((s) => s.method === 'POST' && s.path === '/api/cash-transfers');
    expect(w).toHaveLength(1);
    const lines = (JSON.parse(w[0]!.body ?? '{}') as { lines: { denominationId: number }[] }).lines;
    expect(lines.map((l) => l.denominationId).sort()).toEqual([12, 21]);
  });

  it("CL-5 the picker's label is the page's Currency header and its blank choice asks for a currency (th / en)", async () => {
    const el = await mount();
    const label = form(el).querySelector(`label[for="${picker(el)!.id}"]`);
    expect(label?.textContent?.trim()).toBe(th.headerCurrency);
    expect(picker(el)!.options[0]!.textContent?.trim()).toBe('เลือกสกุลเงิน');
    expect((th as Record<string, string>).chooseCurrency).toBe('เลือกสกุลเงิน');
    expect((en as Record<string, string>).chooseCurrency).toBe('Choose a currency');
  });
});

describe('admin-cash: long cells wrap and the footnote sits in its card', () => {
  it('CL-6 the transfer note and creator cells and the movement reason cell carry .cell-wrap', async () => {
    const el = await mount();
    const tds = [...el.querySelectorAll<HTMLTableCellElement>('.transfers-table tbody td')];
    const note = tds.find((td) => td.textContent?.trim() === LONG_NOTE);
    expect(note?.classList.contains('cell-wrap'), 'transfer note cell').toBe(true);
    const creator = tds.find((td) => td.textContent?.includes('Finance'));
    expect(creator?.classList.contains('cell-wrap'), 'transfer creator cell').toBe(true);
    const moveReason = [...el.querySelectorAll<HTMLTableCellElement>('.movements-table tbody td')].find(
      (td) => td.textContent?.trim() === LONG_NOTE,
    );
    expect(moveReason?.classList.contains('cell-wrap'), 'movement reason cell').toBe(true);
  });

  it('CL-7 the stock footnote is inside the stock card', async () => {
    const el = await mount();
    const notes = [...el.querySelectorAll<HTMLElement>('.footnote')].filter((p) => p.textContent?.trim() === th.footnote);
    expect(notes).toHaveLength(1);
    expect(notes[0]!.closest('.stock-card')).not.toBeNull();
  });
});

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (m[1]!.trim() !== selector) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

describe('admin-cash: cash.css', () => {
  let css = '';
  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    // Resolve next to this file, not process.cwd(): main's web job runs vitest with --root web from the repo root.
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    css = fs.readFileSync(here + 'cash.css', 'utf8');
  });

  it('CL-8 .cell-wrap wraps: white-space normal, overflow-wrap anywhere, min-width 18ch', () => {
    const d = decls(css, '.xc-admin .xc-content .cell-wrap');
    expect(d, 'rule `.xc-admin .xc-content .cell-wrap`').toBeDefined();
    expect(d!.get('white-space')).toBe('normal');
    expect(d!.get('overflow-wrap')).toBe('anywhere');
    expect(d!.get('min-width')).toBe('18ch');
  });

  it("CL-9 the footnote inside the stock card lines up with the card's 22px inset", () => {
    const d = decls(css, '.xc-admin .xc-content .stock-card .footnote');
    expect(d, 'rule `.xc-admin .xc-content .stock-card .footnote`').toBeDefined();
    expect(d!.get('margin')).toBe('0');
    expect(d!.get('padding')).toBe('12px 22px 18px');
  });
});
