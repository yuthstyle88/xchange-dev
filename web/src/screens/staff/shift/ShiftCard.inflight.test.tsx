// #2409: Staff shift card — a second click (or Enter) while a count save, hand-over save, verify,
// accept or reject is still in flight must not send again, must not show an error, and must not
// lose the idempotency key. One test per row of the ticket's behaviour table (part 8), plus the
// edge cases of part 9.
//
// Render and fetch set-up follow the neighbouring `ShiftCard.nolines.test.tsx` and `shift.test.tsx`.
// The fake server models the idempotency store (`api/src/infrastructure/idempotency/pg_store.rs`):
// a write whose key is still PENDING (its first request not answered yet) is refused with 409
// `idempotencyConflict`. A write can be held (answered later), and answered with a network error.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ShiftCard } from './ShiftCard';
import { setLang } from '../../../shared/i18n';
import type { StaffCtx } from '../cards';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { CashCountRow } from '../../../shared/api/generated/cash_counts';
import type { HandoverPendingRow } from '../../../shared/api/generated/handovers_pending';
import type { DenominationRow } from '../../../shared/api/generated/denominations';
import type { HandoverReceiverRow } from '../../../shared/api/generated/handover_receivers';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000a1';
const OTHER = '00000000-0000-4000-8000-0000000000a2';

function me(permissions: string[]): StaffMeView {
  return { userId: ME, name: 'Ploy', roles: [], branchId: BRANCH, branchName: 'XC-B', permissions };
}

function floor(): StaffFloorView {
  return {
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: { id: 'shift-open-001', openedAt: '2026-10-11T01:05:00Z' },
    cutoff: null,
    target: null,
    safe: null,
  };
}

const COUNTS: CashCountRow[] = [
  {
    id: 'count-w-001',
    branchId: BRANCH,
    shiftId: 'shift-other-001',
    kind: 'CLOSE',
    countedBy: OTHER,
    countedAt: '2026-10-11T02:00:00Z',
    verifiedBy: null,
    verifiedAt: null,
    note: null,
    totals: [],
  },
];
const HANDOVERS: HandoverPendingRow[] = [
  {
    id: 'ho-001',
    fromShiftId: 'shift-other-001',
    countId: 'count-ho-001',
    receiverId: ME,
    status: 'PENDING',
    createdAt: '2026-10-11T02:10:00Z',
  },
];
const DENOMS: DenominationRow[] = [
  { id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' },
  { id: 12, currency: 'THB', faceValue: '500.0000', groupCode: 'NOTE' },
];
const RECEIVERS: HandoverReceiverRow[] = [{ userId: OTHER, name: 'Somchai' }];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

interface Sent {
  path: string;
  key: string | null;
  body: string | undefined;
}

type Answer = { status: number; body: unknown } | 'network';

class Server {
  sent: Sent[] = [];
  /** Paths whose next write waits until `release()`. */
  hold = new Set<string>();
  /** Queued answers per path; without one, the write succeeds. */
  answers: Record<string, Answer[]> = {};
  /** Keys whose first request is not answered yet (the store's PENDING rows). */
  private pendingKeys = new Set<string>();
  private waiting: (() => void)[] = [];

  release(): void {
    const w = this.waiting;
    this.waiting = [];
    for (const f of w) f();
  }

  sentTo(path: string): Sent[] {
    return this.sent.filter((s) => s.path === path);
  }

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const path = url.split('?')[0]!;
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET') {
      const views: Record<string, unknown> = {
        cash_counts: COUNTS,
        handovers_pending: HANDOVERS,
        denominations: { rows: DENOMS },
        handover_receivers: { rows: RECEIVERS },
      };
      const name = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
      if (name in views) return json(200, { version: 1, changed: true, data: views[name] });
      return json(404, { error: { code: 'notFound', message: 'not found' } });
    }
    const key = new Headers(init?.headers).get('idempotency-key');
    this.sent.push({ path, key, body: init?.body as string | undefined });
    if (key && this.pendingKeys.has(key)) {
      return json(409, { error: { code: 'idempotencyConflict', message: 'request still in progress' } });
    }
    if (key) this.pendingKeys.add(key);
    try {
      if (this.hold.has(path)) {
        this.hold.delete(path);
        await new Promise<void>((resolve) => this.waiting.push(resolve));
      }
      const next = this.answers[path]?.shift();
      if (next === 'network') throw new TypeError('Failed to fetch');
      if (next) return json(next.status, next.body);
      return json(200, { id: path.split('/')[3] ?? 'new-001', status: 'OK' });
    } finally {
      if (key) this.pendingKeys.delete(key);
    }
  };
}

let server: Server;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

function mount(permissions: string[]): void {
  const ctx: StaffCtx = { me: me(permissions), floor: floor(), refresh: vi.fn(async () => {}) };
  dispose = render(() => <ShiftCard ctx={ctx} />, host);
}

function tid(id: string): string {
  return `[data-testid="${id}"]`;
}

function must<T extends Element = HTMLElement>(sel: string): T {
  const el = host.querySelector<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

function errorShown(): string | null {
  return host.querySelector(tid('shift-error'))?.textContent?.trim() ?? null;
}

function noticeShown(): string | null {
  return host.querySelector(tid('shift-notice'))?.textContent?.trim() ?? null;
}

/** Open the count form and type one quantity. */
async function openCountWith(qty: string): Promise<HTMLFormElement> {
  must<HTMLButtonElement>(tid('shift-close-count')).click();
  await vi.waitFor(() => expect(host.querySelectorAll('input[data-denomination-id]').length).toBe(2));
  const input = must<HTMLInputElement>('input[data-denomination-id="11"]');
  input.value = qty;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  return must<HTMLFormElement>(tid('shift-count-form'));
}

function submit(form: HTMLFormElement): void {
  form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
}

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
});

afterEach(async () => {
  server.release();
  await settle();
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('#2409 Staff shift card: a second click while a write is in flight', () => {
  it('row 1: double click Verify → one POST /api/cash-counts/{id}/verify', async () => {
    const path = '/api/cash-counts/count-w-001/verify';
    mount(['ShiftCloseCertify']);
    await vi.waitFor(() => expect(host.querySelector(tid('shift-verify'))).not.toBeNull());

    server.hold.add(path);
    const verify = must<HTMLButtonElement>(tid('shift-verify'));
    verify.click();
    await settle();
    verify.click();
    await settle();
    expect(server.sentTo(path).length, 'one verify while the first is in flight').toBe(1);
    expect(verify.disabled, 'Verify is disabled while in flight').toBe(true);

    server.release();
    await settle();
    expect(server.sentTo(path).length, 'still one verify').toBe(1);
    expect(errorShown()).toBeNull();
  });

  it('row 2: click Accept while the first accept is in flight → no error; "saved" shows', async () => {
    const path = '/api/handovers/ho-001/accept';
    mount(['ShiftOperate']);
    await vi.waitFor(() => expect(host.querySelector(tid('shift-accept'))).not.toBeNull());

    server.hold.add(path);
    const accept = must<HTMLButtonElement>(tid('shift-accept'));
    accept.click();
    await settle();
    accept.click();
    await settle();
    expect(errorShown(), 'no error while the first accept is in flight').toBeNull();
    expect(server.sentTo(path).length).toBe(1);

    server.release();
    await settle();
    expect(errorShown(), 'no error after it lands').toBeNull();
    expect(noticeShown()).toBe('บันทึกแล้ว');
  });

  it('row 3: network error on count Save (clicked twice), then retry → same key, byte-equal body', async () => {
    const path = '/api/cash-counts';
    mount(['ShiftOperate']);
    await settle();
    const form = await openCountWith('4');
    const save = must<HTMLButtonElement>(tid('shift-count-save'));

    server.hold.add(path);
    server.answers[path] = ['network'];
    save.click();
    await settle();
    save.click();
    await settle();
    server.release();
    await settle();
    expect(errorShown(), 'the network error is shown').not.toBeNull();

    // the retry, once the first send has failed
    submit(must<HTMLFormElement>(tid('shift-count-form')));
    await settle();
    const sent = server.sentTo(path);
    const first = sent[0]!;
    const retry = sent[sent.length - 1]!;
    expect(retry.key, 'the retry reuses the first key').toBe(first.key);
    expect(retry.body, 'byte-equal body').toBe(first.body);
    expect(sent.length, 'the first send and the retry only').toBe(2);
    expect(form.isConnected, 'the count form closed after the retry landed').toBe(false);
  });

  it('edge: Enter twice on the count form while the save is in flight → one POST /api/cash-counts', async () => {
    const path = '/api/cash-counts';
    mount(['ShiftOperate']);
    await settle();
    const form = await openCountWith('2');

    server.hold.add(path);
    submit(form);
    await settle();
    submit(form);
    await settle();
    expect(server.sentTo(path).length).toBe(1);
    expect(must<HTMLButtonElement>(tid('shift-count-save')).disabled, 'Save disabled while in flight').toBe(true);
    expect(errorShown()).toBeNull();

    server.release();
    await settle();
    expect(server.sentTo(path).length).toBe(1);
    expect(noticeShown()).toBe('บันทึกแล้ว');
  });

  it('edge: double click hand-over Save and Reject → one POST each, no error', async () => {
    mount(['ShiftOperate']);
    await vi.waitFor(() => expect(host.querySelector(tid('shift-reject'))).not.toBeNull());

    // reject
    const rejectPath = '/api/handovers/ho-001/reject';
    server.hold.add(rejectPath);
    const reject = must<HTMLButtonElement>(tid('shift-reject'));
    reject.click();
    await settle();
    reject.click();
    await settle();
    expect(server.sentTo(rejectPath).length).toBe(1);
    expect(errorShown()).toBeNull();
    server.release();
    await settle();

    // hand-over save
    const handoverPath = '/api/handovers';
    must<HTMLButtonElement>(tid('shift-handover')).click();
    await vi.waitFor(() => expect(host.querySelectorAll(`${tid('shift-receiver')} option`).length).toBe(2));
    const select = must<HTMLSelectElement>(tid('shift-receiver'));
    select.value = OTHER;
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    server.hold.add(handoverPath);
    const save = must<HTMLButtonElement>(tid('shift-handover-save'));
    save.click();
    await settle();
    save.click();
    await settle();
    expect(server.sentTo(handoverPath).length).toBe(1);
    expect(errorShown()).toBeNull();
    server.release();
    await settle();
    expect(server.sentTo(handoverPath).length).toBe(1);
    expect(noticeShown()).toBe('บันทึกแล้ว');
  });

  it('edge: busy clears after a refusal, so Verify can be pressed again', async () => {
    const path = '/api/cash-counts/count-w-001/verify';
    server.answers[path] = [{ status: 403, body: { error: { code: 'roleNotAllowedHere', message: 'refused' } } }];
    mount(['ShiftCloseCertify']);
    await vi.waitFor(() => expect(host.querySelector(tid('shift-verify'))).not.toBeNull());

    must<HTMLButtonElement>(tid('shift-verify')).click();
    await settle();
    expect(errorShown(), 'the refusal is shown').not.toBeNull();
    const verify = must<HTMLButtonElement>(tid('shift-verify'));
    expect(verify.disabled, 'Verify is enabled again after the refusal').toBe(false);

    verify.click();
    await settle();
    expect(server.sentTo(path).length, 'the second press is sent').toBe(2);
  });
});
