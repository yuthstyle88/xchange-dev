// Survey row A4 (#564), up-front tests by the sub-controller claude/dell-claude-2. 🔴 Do not edit, delete or weaken.
// F01's pending reversals are decided on the sheet through the approval core (executive-finance.md §5
// "reversal approval": AP-1 `decide`, `POST /api/approvals/{id}/decide`). One test per behaviour row.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { warmModules } from '../../../../shared/testing/warm-modules';
import { makeReversalPendingRow, type ReversalPendingRow } from '../../../../shared/testing/fixtures';
import { ReversalsPart } from './ReversalsPart';

beforeAll(() => warmModules(() => import('./ReversalsPart')), 60_000);

const ROWS = [
  makeReversalPendingRow({ requestId: 'req-1', transactionId: 'tx-1', requestedBy: 'u-teller', createdAt: '2026-10-19T02:00:00Z' }),
  makeReversalPendingRow({ requestId: 'req-2', transactionId: 'tx-2', requestedBy: 'u-teller', createdAt: '2026-10-19T03:00:00Z' }),
];

interface Sent {
  path: string;
  body: Record<string, unknown>;
}

describe('A4: F01 pending reversals are approved or rejected on the sheet', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let sent: Sent[];
  let answer: { status: number; body: unknown };
  let release: (() => void) | null;
  let onDecided: ReturnType<typeof vi.fn<() => Promise<void>>>;

  beforeEach(() => {
    setLang('en');
    sent = [];
    release = null;
    answer = { status: 200, body: { status: 'APPROVED' } };
    onDecided = vi.fn<() => Promise<void>>(() => Promise.resolve());
    root = document.createElement('div');
    document.body.appendChild(root);
    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        if ((init?.method ?? 'GET').toUpperCase() === 'POST') {
          sent.push({ path: url.pathname, body: JSON.parse(String(init?.body ?? '{}')) });
        }
        const respond = () =>
          new Response(JSON.stringify(answer.body), {
            status: answer.status,
            headers: { 'content-type': 'application/json' },
          });
        if (release === null) return Promise.resolve(respond());
        return new Promise<Response>((resolve) => {
          const prev = release!;
          release = () => {
            prev();
            resolve(respond());
          };
        });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    vi.unstubAllGlobals();
  });

  function mount(rows: ReversalPendingRow[] = ROWS) {
    dispose = render(() => <ReversalsPart rows={rows} branch="MAIN" onDecided={onDecided} />, root);
  }

  const row = (id: string) => root.querySelector<HTMLElement>(`[data-request="${id}"]`);
  const button = (id: string, action: 'approve' | 'reject') =>
    row(id)?.querySelector<HTMLButtonElement>(`button[data-action="${action}"]`) ?? null;
  const reason = (id: string) => row(id)?.querySelector<HTMLInputElement>('input[name="reason"]') ?? null;
  const settle = () => new Promise((r) => setTimeout(r, 0));

  it('1: every pending row shows its transaction and an approve and a reject button', () => {
    mount();
    for (const r of ROWS) {
      expect(row(r.requestId), r.requestId).not.toBeNull();
      expect(row(r.requestId)!.textContent).toContain(r.transactionId);
      expect(button(r.requestId, 'approve'), `approve ${r.requestId}`).not.toBeNull();
      expect(button(r.requestId, 'reject'), `reject ${r.requestId}`).not.toBeNull();
      expect(reason(r.requestId), `reason ${r.requestId}`).not.toBeNull();
    }
  });

  it('2: approve sends APPROVED for that request on the sheet branch, then re-reads the sheet', async () => {
    mount();
    button('req-1', 'approve')!.click();
    await settle();
    await settle();
    expect(sent).toHaveLength(1);
    expect(sent[0]!.path).toBe('/api/approvals/req-1/decide');
    expect(sent[0]!.body).toMatchObject({ outcome: 'APPROVED', branch: 'MAIN' });
    expect(onDecided).toHaveBeenCalledTimes(1);
  });

  it('3: reject with an empty reason sends nothing and says a reason is required', async () => {
    mount();
    button('req-1', 'reject')!.click();
    await settle();
    expect(sent).toHaveLength(0);
    expect(onDecided).not.toHaveBeenCalled();
    const error = row('req-1')!.querySelector('[data-error]');
    expect(error, 'the row shows why nothing was sent').not.toBeNull();
    expect(error!.textContent!.trim()).not.toBe('');
  });

  it('4: reject with a reason sends REJECTED with that reason, then re-reads the sheet', async () => {
    mount();
    const input = reason('req-2')!;
    input.value = 'customer kept the cash';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    button('req-2', 'reject')!.click();
    await settle();
    await settle();
    expect(sent).toHaveLength(1);
    expect(sent[0]!.path).toBe('/api/approvals/req-2/decide');
    expect(sent[0]!.body).toMatchObject({ outcome: 'REJECTED', reason: 'customer kept the cash', branch: 'MAIN' });
    expect(onDecided).toHaveBeenCalledTimes(1);
  });

  it('5: a refused decision shows the worded error on that row and does not re-read', async () => {
    answer = { status: 403, body: { error: { code: 'selfDecision', message: 'You cannot decide your own request' } } };
    mount();
    button('req-1', 'approve')!.click();
    await settle();
    await settle();
    expect(sent).toHaveLength(1);
    expect(onDecided).not.toHaveBeenCalled();
    const error = row('req-1')!.querySelector('[data-error]');
    expect(error).not.toBeNull();
    expect(error!.textContent!.trim()).not.toBe('');
    expect(row('req-2')!.querySelector('[data-error]'), 'only the refused row').toBeNull();
  });

  it('6: a double click on approve sends one decision', async () => {
    release = () => {};
    mount();
    button('req-1', 'approve')!.click();
    button('req-1', 'approve')!.click();
    await settle();
    expect(sent).toHaveLength(1);
    release!();
    await settle();
    await settle();
    expect(onDecided).toHaveBeenCalledTimes(1);
  });

  it('7: no pending reversals shows the empty line (kept)', () => {
    mount([]);
    expect(root.querySelector('[data-request]')).toBeNull();
    expect(root.querySelector('.reconciliation-none')).not.toBeNull();
  });
});
