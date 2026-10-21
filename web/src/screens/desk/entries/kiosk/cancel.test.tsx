// KC-C (kiosk-cancel FE, docs/design/blueprint/kiosk-cancel.md §3.5 "The cashier screen", §5 row KC-C): the desk's
// cancel of an open kiosk request, and the entry bar that shows an entry's banner and actions. Written before the
// implementation by the sub-controller claude/36d9dc6a. Do not edit, delete or weaken these tests; add tests in a
// file of your own.
//
// What these tests pin (on top of KC-R's CancelReasonDialog and cancelKioskRequest):
// - `DeskEntryState` gains `actions?: () => JSX.Element`. The kiosk entry's `actions()` renders
//   button[data-action="cancel-kiosk-request"] labelled `cancelRequest` from ./i18n (th "ยกเลิกคำขอ") while the
//   store's `outcome()` is not 'saved', and no such button once it is.
// - The button opens CancelReasonDialog (dialog.xc-dialog). Confirm posts POST /api/kiosk
//   {action: "cancel", payload: {id, branchId: store.branch(), reason, note?}}.
// - On success the desk loads `/desk?branch=<branch>` with `window.location.assign` (no `?kiosk=`), never
//   `startNew()` (its unsaved confirm would ask about the kiosk's own amount, desk-store.ts:428-434).
// - On a refusal (400 {"error": sentence}) the sentence shows (role="alert") and the page does not move.
// - `DeskPage` renders a bar `[data-desk-entry-bar]` with the entry's `banner()` text and its `actions?.()` whenever
//   `store.entry()` is set, and no bar when it is not. Nothing renders `banner` on `main` before this ticket.
// Wire shape and sentences: AG controller KC, #564 issuecomment-6307996084.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { DeskPage } from '../../DeskPage';
import type { DeskEntryState, SaveOutcome } from '../../desk-store';
import { entry } from './entry';
import { th } from './i18n';

const GET_ANSWER = {
  paymentMethod: 'Cash',
  type: 'BUY',
  currency: 'USD',
  rate: 32.1,
  foreign: 100,
  thb: 3210,
  inputAmount: 100,
  branchName: 'XC-B',
  id: 'k-7',
  queue: 'K007',
};

function fakeServer(cancel: () => { status: number; body: unknown } = () => ({
  status: 200,
  body: { id: 'k-7', queue: 'K007', orderNo: null, transactionNo: null, status: 'CANCELLED' },
})) {
  const calls: { path: string; body: any }[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input).split('?')[0];
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ path, body });
    const a =
      path === '/api/kiosk' && body?.action === 'get'
        ? { status: 200, body: GET_ANSWER }
        : path === '/api/kiosk' && body?.action === 'cancel'
          ? cancel()
          : { status: 404, body: { error: { code: 'notFound', message: 'nf' } } };
    return new Response(JSON.stringify(a.body), { status: a.status, headers: { 'content-type': 'application/json' } });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), calls };
}

function stubStore(api: ApiClient, outcome: Accessor<SaveOutcome>) {
  return {
    api,
    branch: () => 'MAIN',
    outcome,
    identityChecked: () => true,
    setType: vi.fn(),
    setCurrency: vi.fn(),
    setAmount: vi.fn(),
    setMethod: vi.fn(),
    startNew: vi.fn(() => true),
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

function text(node: Element | null | undefined): string {
  return (node?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let assign: ReturnType<typeof vi.fn>;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  assign = vi.fn();
  vi.stubGlobal('location', { ...window.location, assign, search: '?branch=MAIN&kiosk=k-7', pathname: '/desk' });
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function loadedActions(outcome: Accessor<SaveOutcome>, server = fakeServer()) {
  const store = stubStore(server.api, outcome);
  const state = (await entry.load(store as any, 'k-7', server.api)) as DeskEntryState & {
    actions?: () => unknown;
  };
  expect(typeof state.actions, 'the kiosk entry has actions()').toBe('function');
  dispose = render(() => state.actions!() as any, host);
  return { store, state, server };
}

function cancelButton(): HTMLButtonElement | null {
  return host.querySelector<HTMLButtonElement>('button[data-action="cancel-kiosk-request"]');
}

async function confirmIn(reason: string): Promise<void> {
  const d = document.querySelector<HTMLDialogElement>('dialog.xc-dialog');
  if (!d) throw new Error('CancelReasonDialog did not open');
  const sel = d.querySelector<HTMLSelectElement>('select[name="reason"]')!;
  sel.value = reason;
  sel.dispatchEvent(new Event('change', { bubbles: true }));
  d.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel-request"]')!.click();
  await flush();
}

describe('KC-C: the kiosk entry cancel (kiosk-cancel.md §3.5)', () => {
  it('row 1: actions() shows a "cancelRequest" button while the save is not done', async () => {
    const [outcome] = createSignal<SaveOutcome>('idle');
    await loadedActions(outcome);
    const b = cancelButton();
    expect(b, 'button[data-action="cancel-kiosk-request"]').not.toBeNull();
    expect(text(b)).toBe(th['cancelRequest' as keyof typeof th]);
    expect(th['cancelRequest' as keyof typeof th]).toBe('ยกเลิกคำขอ');
  });

  it('row 2: once the save completes the cancel button is gone', async () => {
    const [outcome, setOutcome] = createSignal<SaveOutcome>('idle');
    await loadedActions(outcome);
    expect(cancelButton()).not.toBeNull();
    setOutcome('saved');
    await flush();
    expect(cancelButton()).toBeNull();
  });

  it('row 3: confirming posts the cancel for this request and branch, then loads /desk?branch=MAIN without startNew', async () => {
    const [outcome] = createSignal<SaveOutcome>('idle');
    const { store, server } = await loadedActions(outcome);
    cancelButton()!.click();
    await flush();
    await confirmIn('CUSTOMER_CHANGED_MIND');
    const cancels = server.calls.filter((c) => c.body?.action === 'cancel');
    expect(cancels).toHaveLength(1);
    expect(cancels[0]).toEqual({
      path: '/api/kiosk',
      body: { action: 'cancel', payload: { id: 'k-7', branchId: 'MAIN', reason: 'CUSTOMER_CHANGED_MIND' } },
    });
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/desk?branch=MAIN');
    expect(store.startNew).not.toHaveBeenCalled();
  });

  it('row 4: a refusal shows the server sentence and the page does not move', async () => {
    const [outcome] = createSignal<SaveOutcome>('idle');
    await loadedActions(
      outcome,
      fakeServer(() => ({ status: 400, body: { error: 'คำขอนี้ยกเลิกไม่ได้' } })),
    );
    cancelButton()!.click();
    await flush();
    await confirmIn('DUPLICATE');
    expect(text(document.querySelector('[role="alert"]'))).toContain('คำขอนี้ยกเลิกไม่ได้');
    expect(assign).not.toHaveBeenCalled();
  });
});

describe('KC-C: DeskPage renders the entry bar (banner + actions)', () => {
  function pageStore(state: DeskEntryState | undefined) {
    const [step] = createSignal(0 as 0);
    return { step, go: vi.fn(), entry: () => state };
  }

  it('row 5: with an entry, a [data-desk-entry-bar] shows its banner and its actions', () => {
    const state = {
      kind: 'kiosk',
      id: 'k-7',
      banner: () => 'ข้อมูลจากคิว K007',
      actions: () => <button type="button" data-action="probe-action">probe</button>,
    } as unknown as DeskEntryState;
    dispose = render(() => <DeskPage store={pageStore(state) as any} steps={[]} />, host);
    const bar = host.querySelector('[data-desk-entry-bar]');
    expect(bar, '[data-desk-entry-bar]').not.toBeNull();
    expect(text(bar)).toContain('ข้อมูลจากคิว K007');
    expect(bar!.querySelector('button[data-action="probe-action"]')).not.toBeNull();
  });

  it('row 6: without an entry there is no bar', () => {
    dispose = render(() => <DeskPage store={pageStore(undefined) as any} steps={[]} />, host);
    expect(host.querySelector('[data-desk-entry-bar]')).toBeNull();
  });
});
