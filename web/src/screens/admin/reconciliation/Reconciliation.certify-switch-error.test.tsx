import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Reconciliation } from './Reconciliation';
import * as table from './i18n';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';

const VIEW = '/api/v3/views/finance_reconciliation';
const CERTIFY = '/api/v3/commands/reconciliation_certify';
const B01 = 'XC-B01';
const B02 = 'XC-B02';
const DAY_1 = '2026-10-05';
const DAY_2 = '2026-10-06';

interface DeferredResponse {
  branch: string;
  date: string;
  resolve: (status?: number, body?: unknown) => void;
}

describe('Reconciliation certify switch error guard (#2895)', () => {
  let root: HTMLElement | undefined;
  let dispose: (() => void) | undefined;
  let pendingReads: DeferredResponse[];
  let pendingCertifies: Array<{
    body: Record<string, unknown> | undefined;
    resolve: (status?: number, body?: unknown) => void;
  }>;

  function waiting(reason: string) {
    return { available: false, reason, data: null };
  }

  function makeSheet(branch: string, date: string, hash: string, summary: string) {
    return {
      branch,
      date,
      transactions: {
        available: true,
        reason: null,
        data: {
          byMethod: [
            {
              method: 'Cash',
              buyThb: '1300.00',
              sellThb: '2000.00',
              fees: '30.00',
              bankFees: '0.00',
              count: 3,
            },
          ],
          reversedThb: '250.00',
        },
      },
      counts: waiting('CA-3'),
      transfers: waiting('CA-5'),
      exceptions: {
        available: true,
        reason: null,
        data: [{ id: `EX-${branch}-${date}`, position: 1, reviewed: false, summary }],
      },
      integrity: waiting('LG-7'),
      reversalsPending: waiting('RV-1'),
      figuresHash: hash,
      certifications: [],
    };
  }

  function defaultSheetFor(branch: string, date: string) {
    const hash = `hash-${branch}-${date}`;
    const summary = `SUMMARY-${branch}-${date}`;
    return makeSheet(branch, date, hash, summary);
  }

  function jsonResponse(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  async function settle(): Promise<void> {
    for (let round = 0; round < 4; round++) {
      for (let i = 0; i < 5; i++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  beforeEach(() => {
    setLang('th');
    toast('');
    pendingReads = [];
    pendingCertifies = [];

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const path = url.pathname;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (path === VIEW && method === 'GET') {
          const branch = url.searchParams.get('branch') ?? '';
          const date = url.searchParams.get('date') ?? '';
          return new Promise<Response>((resolve) => {
            pendingReads.push({
              branch,
              date,
              resolve: (status = 200, body?: unknown) =>
                resolve(
                  jsonResponse(
                    status,
                    body ?? {
                      version: 1,
                      changed: true,
                      data: defaultSheetFor(branch, date),
                    },
                  ),
                ),
            });
          });
        }

        if (path === CERTIFY && method === 'POST') {
          const body =
            typeof init?.body === 'string' && init.body !== ''
              ? (JSON.parse(init.body) as Record<string, unknown>)
              : undefined;
          return new Promise<Response>((resolve) => {
            pendingCertifies.push({
              body,
              resolve: (status = 200, respBody?: unknown) =>
                resolve(
                  jsonResponse(
                    status,
                    respBody ?? {
                      id: 'C-1',
                      certifiedBy: 'u-fin',
                      certifiedAt: '2026-10-16T10:00:00Z',
                      note: String(body?.note ?? ''),
                      figuresHash: String(body?.figuresHash ?? ''),
                      stale: false,
                    },
                  ),
                ),
            });
          });
        }

        return Promise.resolve(jsonResponse(200, { version: 1, changed: false }));
      }),
    );
  });

  afterEach(async () => {
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
    await settle();
  });

  function mount(initialBranch = B01, initialDay = DAY_1) {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period, setPeriod] = createSignal({ from: initialDay, to: initialDay });
    const [shown] = createSignal(true);
    const el = document.createElement('div');
    root = el;
    document.body.appendChild(el);
    dispose = render(
      () =>
        (
          <>
            <Reconciliation branch={branch} period={period} shown={shown} />
            <Dialog />
            <Toast />
          </>
        ) as JSX.Element,
      el,
    );
    return {
      el,
      setBranch,
      setDay: (d: string) => setPeriod({ from: d, to: d }),
    };
  }

  function findCertifyButton(el: ParentNode): HTMLButtonElement | null {
    const btns = [...el.querySelectorAll<HTMLButtonElement>('button')].filter(
      (b) => b.textContent?.trim() === table.th.certify,
    );
    return btns[0] ?? null;
  }

  it('Row 1: Certify on branch A in flight, switch to branch B, A\'s certify fails: no .reconciliation-error on branch B', async () => {
    const { el, setBranch } = mount(B01, DAY_1);
    expect(pendingReads.length).toBe(1);
    pendingReads[0]!.resolve();
    await settle();

    const certifyBtn = findCertifyButton(el)!;
    expect(certifyBtn).not.toBeNull();
    certifyBtn.click();
    await settle();
    expect(pendingCertifies.length).toBe(1);

    // Switch branch to B02 while B01 certify is in flight
    setBranch(B02);
    await settle();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe(B02);

    // B02 read resolves
    pendingReads[1]!.resolve();
    await settle();

    // Now B01 certify fails
    pendingCertifies[0]!.resolve(409, { error: 'staleVersion', message: 'Stale version' });
    await settle();

    // Required: no .reconciliation-error on branch B
    expect(el.querySelector('.reconciliation-error')).toBeNull();
  });

  it('Row 2: Certify on day D1 in flight, switch to day D2 (same branch), D1\'s certify fails: no .reconciliation-error on D2', async () => {
    const { el, setDay } = mount(B01, DAY_1);
    expect(pendingReads.length).toBe(1);
    pendingReads[0]!.resolve();
    await settle();

    const certifyBtn = findCertifyButton(el)!;
    expect(certifyBtn).not.toBeNull();
    certifyBtn.click();
    await settle();
    expect(pendingCertifies.length).toBe(1);

    // Switch day to DAY_2 while DAY_1 certify is in flight
    setDay(DAY_2);
    await settle();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.date).toBe(DAY_2);

    // DAY_2 read resolves
    pendingReads[1]!.resolve();
    await settle();

    // Now DAY_1 certify fails
    pendingCertifies[0]!.resolve(409, { error: 'staleVersion', message: 'Stale version' });
    await settle();

    // Required: no .reconciliation-error on D2
    expect(el.querySelector('.reconciliation-error')).toBeNull();
  });

  it('Row 3: Certify on branch A in flight, stay on A, it fails: .reconciliation-error shows the worded error on A', async () => {
    const { el } = mount(B01, DAY_1);
    expect(pendingReads.length).toBe(1);
    pendingReads[0]!.resolve();
    await settle();

    const certifyBtn = findCertifyButton(el)!;
    expect(certifyBtn).not.toBeNull();
    certifyBtn.click();
    await settle();
    expect(pendingCertifies.length).toBe(1);

    // Stay on branch A and day D1; certify fails
    pendingCertifies[0]!.resolve(409, { error: 'staleVersion', message: 'Stale version' });
    await settle();

    // Required: .reconciliation-error shows the worded error on A
    const errEl = el.querySelector('.reconciliation-error');
    expect(errEl).not.toBeNull();
    expect(errEl?.textContent).toBeTruthy();
  });

  it('Row 4: Certify on A in flight, switch to B, A\'s certify succeeds: B\'s sheet is shown (the #2309 behaviour)', async () => {
    const { el, setBranch } = mount(B01, DAY_1);
    expect(pendingReads.length).toBe(1);
    pendingReads[0]!.resolve();
    await settle();

    const certifyBtn = findCertifyButton(el)!;
    expect(certifyBtn).not.toBeNull();
    certifyBtn.click();
    await settle();
    expect(pendingCertifies.length).toBe(1);

    // Switch branch to B02 while B01 certify is in flight
    setBranch(B02);
    await settle();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe(B02);

    // B02 read resolves
    pendingReads[1]!.resolve();
    await settle();

    // B01 certify succeeds
    pendingCertifies[0]!.resolve();
    await settle();
    while (pendingReads.length > 2) {
      const nextRead = pendingReads.pop()!;
      nextRead.resolve();
      await settle();
    }

    // Required: B's sheet is shown, not A's
    expect(el.textContent).toContain(`SUMMARY-${B02}-${DAY_1}`);
    expect(el.textContent).not.toContain(`SUMMARY-${B01}-${DAY_1}`);
  });
});
