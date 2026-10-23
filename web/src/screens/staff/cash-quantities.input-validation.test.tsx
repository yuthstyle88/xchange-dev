// #4910 (audit T10): cash quantities are checked at the row before anything is sent, and a server
// refusal naming a line (`lines[i].qty`, contract from #4901) shows at that row.
//
// "A line at a row" = an element carrying a `data-field-error` attribute inside the row that holds
// the quantity input: `[data-denomination-row]` on the staff cards, the input's own wrapper
// (`input.parentElement`) on the admin cash flows form. A line at the destination select is
// `[data-field-error="toBranch"]`. "Worded" = the line has text, in Thai while the screen is Thai.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import {
  makeAdminShiftRow,
  makeCashTransferLineRow,
  makeCashTransferRow,
  makeDenominationRow,
  makeDenominations,
  makeMe,
  makeStaffFloorMyShift,
  makeStaffFloorView,
  makeStaffMeView,
  makeTransferBranchRow,
  makeTransferBranches,
  makeV3ErrorBody,
  makeV3ErrorDetail,
  makeViewAnswer,
} from '../../shared/testing/fixtures';
import { Dialog, closeDialog } from '../admin/parts/Dialog';
import { Toast } from '../admin/parts/toast';
import { CashFlows } from '../admin/cash/CashFlows';
import type { StaffCtx } from './cards';
import { ShiftCard } from './shift/ShiftCard';
import { TransfersCard } from './transfers/TransfersCard';

const BRANCH = 'MAIN';
const ELSEWHERE = 'B09';
const ME = '00000000-0000-4000-8000-0000000000c1';
const THAI = /[฀-๿]/;

const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '500.0000', groupCode: 'NOTE' }),
];

const DISPATCHED = makeCashTransferRow({
  id: 'tr-in-1',
  kind: 'BRANCH',
  status: 'DISPATCHED',
  fromBranchId: ELSEWHERE,
  toBranchId: BRANCH,
  lines: [makeCashTransferLineRow({ currency: 'THB', denominationId: 11, qtySent: '3' })],
});

interface Sent {
  method: string;
  path: string;
  body: string;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let sent: Sent[];
let answers: Record<string, { status: number; body: unknown }>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stub(views: Record<string, unknown>) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(raw, 'http://localhost').pathname;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const name = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
        if (name in views) return json(200, makeViewAnswer(views[name]));
        return json(404, makeV3ErrorBody({ error: makeV3ErrorDetail({ code: 'notFound', message: 'not found' }) }));
      }
      sent.push({ method, path, body: typeof init?.body === 'string' ? init.body : '' });
      const answer = answers[path];
      if (answer) return json(answer.status, answer.body);
      return json(200, { id: 'rec-1', status: 'DRAFT' });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

function staffMe(permissions: string[]) {
  return makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function staffCtx(permissions: string[]): StaffCtx {
  return {
    me: staffMe(permissions),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: '2026-10-11T01:00:00Z',
      myShift: makeStaffFloorMyShift({ id: 'shift-open-001', openedAt: '2026-10-11T01:05:00Z' }),
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  };
}

function type(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function choose(select: HTMLSelectElement, value: string) {
  select.value = value;
  select.dispatchEvent(new Event('input', { bubbles: true }));
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

function must<T extends Element>(sel: string, from: ParentNode = host): T {
  const el = from.querySelector<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

/** The worded line inside a container, or null. */
function lineIn(container: Element | null): Element | null {
  return container?.querySelector('[data-field-error]') ?? null;
}

function expectWorded(line: Element | null, where: string) {
  expect(line, `a line at ${where}`).not.toBeNull();
  const text = line!.textContent?.trim() ?? '';
  expect(text, `the line at ${where} has text`).not.toBe('');
  expect(text, `the line at ${where} is worded in Thai`).toMatch(THAI);
}

function staffRow(form: Element, denominationId: number): Element {
  const input = must<HTMLInputElement>(`input[data-denomination-id="${denominationId}"]`, form);
  const row = input.closest('[data-denomination-row]');
  expect(row, `row of denomination ${denominationId}`).not.toBeNull();
  return row!;
}

function posts(path: string): Sent[] {
  return sent.filter((s) => s.method === 'POST' && s.path === path);
}

beforeEach(() => {
  setLang('th');
  sent = [];
  answers = {};
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('#4910 cash quantities: checked at the row before sending', () => {
  async function openShiftCount(): Promise<HTMLFormElement> {
    stub({
      cash_counts: [],
      handovers_pending: [],
      denominations: { rows: DENOMS },
      handover_receivers: { rows: [] },
    });
    const ctx = staffCtx(['ShiftOperate']);
    dispose = render(() => <ShiftCard ctx={ctx} />, host);
    await settle();
    must<HTMLButtonElement>('[data-testid="shift-close-count"]').click();
    await settle();
    return must<HTMLFormElement>('[data-testid="shift-count-form"]');
  }

  async function submit(form: HTMLFormElement) {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
  }

  it('Q1: shift count row "1.5" shows a worded line at that row and sends nothing', async () => {
    const form = await openShiftCount();
    type(must<HTMLInputElement>('input[data-denomination-id="11"]', form), '1.5');
    await submit(form);

    expect(posts('/api/cash-counts'), 'no count is sent').toEqual([]);
    expectWorded(lineIn(staffRow(form, 11)), 'row THB 1000');
    expect(lineIn(staffRow(form, 12)), 'the untouched row has no line').toBeNull();
  });

  it('Q2: transfer create row "0" shows a worded line at that row and sends nothing', async () => {
    stub({
      cash_transfers: [],
      denominations: makeDenominations({ rows: DENOMS }),
      transfer_branches: makeTransferBranches({ rows: [makeTransferBranchRow({ branchId: ELSEWHERE, name: 'Ayutthaya' })] }),
    });
    const ctx = staffCtx(['CashManage']);
    dispose = render(() => <TransfersCard ctx={ctx} />, host);
    await settle();
    must<HTMLButtonElement>('[data-testid="transfer-new"]').click();
    await settle();
    const form = must<HTMLFormElement>('[data-testid="transfer-form"]');
    choose(must<HTMLSelectElement>('[data-testid="transfer-kind"]', form), 'BRANCH');
    await settle();
    choose(must<HTMLSelectElement>('[data-testid="transfer-to-branch"]', form), ELSEWHERE);
    type(must<HTMLInputElement>('input[data-denomination-id="12"]', form), '0');
    await settle();
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(posts('/api/cash-transfers'), 'no transfer is created').toEqual([]);
    expectWorded(lineIn(staffRow(form, 12)), 'row THB 500');
  });

  it('Q3: transfer receive row "0" is accepted and sent as qtyReceived "0"', async () => {
    stub({
      cash_transfers: [DISPATCHED],
      denominations: makeDenominations({ rows: DENOMS }),
      transfer_branches: makeTransferBranches({ rows: [] }),
    });
    const ctx = staffCtx(['CashManage']);
    dispose = render(() => <TransfersCard ctx={ctx} />, host);
    await settle();
    must<HTMLButtonElement>('[data-testid="transfer-receive"]').click();
    await settle();
    const form = must<HTMLFormElement>('[data-testid="transfer-receive-form"]');
    type(must<HTMLInputElement>('input[data-denomination-id="11"]', form), '0');
    await settle();
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    const received = posts('/api/cash-transfers/tr-in-1/receive');
    expect(received.length, 'the receive is sent once').toBe(1);
    const body = JSON.parse(received[0]!.body) as { lines: { denominationId: number; qtyReceived: string }[] };
    expect(body.lines).toEqual([expect.objectContaining({ denominationId: 11, qtyReceived: '0' })]);
    expect(lineIn(staffRow(form, 11)), 'no line at an accepted row').toBeNull();
  });

  it('Q5: a quantity over the i32 limit ("3000000000") shows a worded line at that row and sends nothing', async () => {
    const form = await openShiftCount();
    type(must<HTMLInputElement>('input[data-denomination-id="12"]', form), '3000000000');
    await submit(form);

    expect(posts('/api/cash-counts'), 'no count is sent').toEqual([]);
    expectWorded(lineIn(staffRow(form, 12)), 'row THB 500');
  });
});

describe('#4910 admin cash flows: destination and server refusals at the field', () => {
  async function mountFlows(): Promise<HTMLFormElement> {
    stub({
      me: makeMe({
        id: 'u-admin',
        name: 'Admin',
        roles: [],
        permissions: ['ShiftCloseCertify', 'CashManage', 'FinanceReconcile'],
        branch: BRANCH,
        branchName: 'Headquarters',
      }),
      cash_counts: [],
      cash_transfers: [],
      denominations: makeDenominations({ rows: DENOMS }),
      transfer_branches: makeTransferBranches({ rows: [makeTransferBranchRow({ branchId: ELSEWHERE, name: 'Ayutthaya' })] }),
    });
    const [branch] = createSignal(BRANCH);
    const [shown] = createSignal(true);
    const [shifts] = createSignal([makeAdminShiftRow({ id: 'shift-open-001', status: 'OPEN', staff: 'Ploy' })]);
    dispose = render(
      () => (
        <>
          <CashFlows branch={branch} shown={shown} shifts={shifts} />
          <Dialog />
          <Toast />
        </>
      ),
      host,
    );
    await settle();
    return must<HTMLFormElement>('form.cash-new-transfer');
  }

  function adminRow(form: Element, denominationId: number): Element {
    const input = must<HTMLInputElement>(`input[data-denomination="${denominationId}"]`, form);
    expect(input.parentElement, `wrapper of denomination ${denominationId}`).not.toBeNull();
    return input.parentElement!;
  }

  it('Q4: a BRANCH transfer with no destination shows a worded line at the destination and sends nothing', async () => {
    const form = await mountFlows();
    choose(must<HTMLSelectElement>('#transfer-kind', form), 'BRANCH');
    await settle();
    type(must<HTMLInputElement>('input[data-denomination="11"]', form), '5');
    await settle();
    must<HTMLButtonElement>('button[data-action="create"]', form).click();
    await settle();

    expect(posts('/api/cash-transfers'), 'no transfer is created').toEqual([]);
    expectWorded(form.querySelector('[data-field-error="toBranch"]'), 'the destination');
  });

  it('Q6: a server 400 on field lines[1].qty shows the line at the second sent row, never the server text', async () => {
    answers['/api/cash-transfers'] = {
      status: 400,
      body: makeV3ErrorBody({
        error: makeV3ErrorDetail({ code: 'validationFailed', field: 'lines[1].qty', message: 'qty is too large' }),
      }),
    };
    const form = await mountFlows();
    choose(must<HTMLSelectElement>('#transfer-kind', form), 'EXTERNAL_OUT');
    await settle();
    type(must<HTMLInputElement>('input[data-denomination="11"]', form), '2');
    type(must<HTMLInputElement>('input[data-denomination="12"]', form), '4');
    await settle();
    must<HTMLButtonElement>('button[data-action="create"]', form).click();
    await settle();

    expect(posts('/api/cash-transfers').length, 'the create is sent once').toBe(1);
    const second = lineIn(adminRow(form, 12));
    expectWorded(second, 'row THB 500 (lines[1])');
    expect(second!.textContent, 'the line names the row').toContain('500');
    expect(lineIn(adminRow(form, 11)), 'the first row has no line').toBeNull();
    expect(host.textContent ?? '', 'the raw server text is never shown').not.toContain('qty is too large');
  });
});
