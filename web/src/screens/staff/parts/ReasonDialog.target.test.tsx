// #2458: Switching the active hold (Release A -> Release B) or transfer (Reconcile X -> Reconcile Y)
// while ReasonDialog is open must clear the previous item's typed note so Confirm never records A's note against B.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeCashTransferRow,
  makeStaffFloorView,
  makeStaffHoldRow,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import type { StaffCtx } from '../cards';
import { HoldsCard } from '../holds/HoldsCard';
import { th as staffTh } from '../i18n';
import { TransfersCard } from '../transfers/TransfersCard';
import { ReasonDialog } from './ReasonDialog';

const BRANCH = 'MAIN';
const ELSEWHERE = 'B09';
const HOLD_A = 'h-open-a';
const HOLD_B = 'h-open-b';
const TRANSFER_X = 'e0000000-0000-4000-8000-000000000010';
const TRANSFER_Y = 'e0000000-0000-4000-8000-000000000011';

const HOLDS = [
  makeStaffHoldRow({
    id: HOLD_A,
    customerId: 'CX-00000001',
    customerName: 'Somchai Jaidee',
    reason: 'AML check A',
    status: 'OPEN',
    at: '2026-10-11T02:00:00Z',
  }),
  makeStaffHoldRow({
    id: HOLD_B,
    customerId: 'CX-00000002',
    customerName: 'Suda Jaidee',
    reason: 'AML check B',
    status: 'OPEN',
    at: '2026-10-11T03:00:00Z',
  }),
];

function receivedTransfer(id: string) {
  return makeCashTransferRow({
    id,
    kind: 'BRANCH',
    fromBranchId: ELSEWHERE,
    toBranchId: BRANCH,
    fromShiftId: null,
    toShiftId: null,
    status: 'RECEIVED',
    createdBy: '00000000-0000-4000-8000-0000000000b2',
    note: 'dispatched cash',
    createdAt: '2026-10-11T02:00:00Z',
    lines: [],
  });
}

const TRANSFERS = [
  receivedTransfer(TRANSFER_X),
  receivedTransfer(TRANSFER_Y),
];

function me(permissions: string[]) {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000c1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function floor() {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewResponse(data: unknown): Response {
  return jsonResponse(200, { version: 1, changed: true, data });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function appendText(input: HTMLInputElement, text: string): void {
  input.value = `${input.value}${text}`;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

interface RecordedPost {
  path: string;
  body: unknown;
}

describe('#2458 ReasonDialog target switch tests', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let posts: RecordedPost[];

  beforeEach(() => {
    setLang('th');
    posts = [];
    host = document.createElement('div');
    document.body.appendChild(host);

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl =
          typeof input === 'string'
            ? input
            : input instanceof URL
              ? input.href
              : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_holds') {
          return viewResponse(HOLDS);
        }
        if (method === 'GET' && path === '/api/v3/views/cash_transfers') {
          return viewResponse(TRANSFERS);
        }
        if (method === 'GET' && path === '/api/v3/views/denominations') {
          return viewResponse({ rows: [] });
        }
        if (method === 'GET' && path === '/api/v3/views/transfer_branches') {
          return viewResponse({ rows: [] });
        }
        if (method === 'POST') {
          const rawBody = init?.body as string | undefined;
          posts.push({
            path,
            body: rawBody ? JSON.parse(rawBody) : undefined,
          });
          return jsonResponse(200, { id: 'ok', status: 'RECONCILED', success: true });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function mountHoldsCard(): Promise<void> {
    const ctx: StaffCtx = {
      me: me(['VerificationReview']),
      floor: floor(),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();
  }

  async function mountTransfersCard(): Promise<void> {
    const ctx: StaffCtx = {
      me: me(['CashManage', 'FinanceReconcile']),
      floor: floor(),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <TransfersCard ctx={ctx} />, host);
    await flush();
  }

  function releaseButton(holdId: string): HTMLButtonElement {
    const btn = host.querySelector<HTMLButtonElement>(
      `[data-hold-id="${holdId}"] [data-testid="holds-release"]`,
    );
    expect(btn, `release button for ${holdId}`).not.toBeNull();
    return btn!;
  }

  function reconcileButton(transferId: string): HTMLButtonElement {
    const btn = host.querySelector<HTMLButtonElement>(
      `[data-transfer-id="${transferId}"] [data-testid="transfer-reconcile"]`,
    );
    expect(btn, `reconcile button for ${transferId}`).not.toBeNull();
    return btn!;
  }

  function dialogInput(): HTMLInputElement {
    const input = host.querySelector<HTMLInputElement>('dialog input');
    expect(input, 'dialog input').not.toBeNull();
    return input!;
  }

  function dialogConfirmButton(): HTMLButtonElement {
    const btn = [...host.querySelectorAll<HTMLButtonElement>('dialog button')].find(
      (b) => b.textContent === staffTh.confirm,
    );
    expect(btn, 'dialog confirm button').not.toBeNull();
    return btn!;
  }

  it('row 1: Release A, type note, click Release B -> Dialog input is empty', async () => {
    await mountHoldsCard();

    releaseButton(HOLD_A).click();
    await flush();
    appendText(dialogInput(), 'inspection note for A');
    expect(dialogInput().value).toBe('inspection note for A');

    releaseButton(HOLD_B).click();
    await flush();
    expect(dialogInput().value).toBe('');
  });

  it("row 2: Then Confirm -> POST for B carries B's note only", async () => {
    await mountHoldsCard();

    releaseButton(HOLD_A).click();
    await flush();
    appendText(dialogInput(), 'note from A ');

    releaseButton(HOLD_B).click();
    await flush();
    appendText(dialogInput(), 'note for B');

    dialogConfirmButton().click();
    await flush();

    expect(posts).toEqual([
      {
        path: `/api/customer-holds/${HOLD_B}/release`,
        body: { id: HOLD_B, note: 'note for B' },
      },
    ]);
  });

  it('row 3: Reconcile X, type note, click Reconcile Y -> Dialog input is empty', async () => {
    await mountTransfersCard();

    reconcileButton(TRANSFER_X).click();
    await flush();
    appendText(dialogInput(), 'reconcile note for X');
    expect(dialogInput().value).toBe('reconcile note for X');

    reconcileButton(TRANSFER_Y).click();
    await flush();
    expect(dialogInput().value).toBe('');
  });

  it('edge 1: Same target clicked again while open keeps the typed note', async () => {
    await mountHoldsCard();

    releaseButton(HOLD_A).click();
    await flush();
    appendText(dialogInput(), 'still typing for A');
    expect(dialogInput().value).toBe('still typing for A');

    releaseButton(HOLD_A).click();
    await flush();
    expect(dialogInput().value).toBe('still typing for A');
  });

  it('edge 2: Caller with no target change has unchanged open/type/confirm/reopen behaviour', () => {
    const [open, setOpen] = createSignal(false);
    const onConfirm = vi.fn();
    dispose = render(
      () => (
        <ReasonDialog
          open={open()}
          title="Standalone dialog"
          onConfirm={(r) => {
            onConfirm(r);
            setOpen(false);
          }}
          onCancel={() => setOpen(false)}
        />
      ),
      host,
    );

    setOpen(true);
    appendText(dialogInput(), 'standalone reason');
    dialogConfirmButton().click();
    expect(onConfirm).toHaveBeenCalledWith('standalone reason');

    setOpen(true);
    expect(dialogInput().value).toBe('');
  });
});
