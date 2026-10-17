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

interface SentCommand {
  path: string;
  key: string | null;
  body: Record<string, unknown> | undefined;
}

describe('Reconciliation branch and day switch guard (#2309)', () => {
  let root: HTMLElement | undefined;
  let dispose: (() => void) | undefined;
  let pendingReads: DeferredResponse[];
  let pendingCertifies: Array<{
    body: Record<string, unknown> | undefined;
    resolve: (status?: number, body?: unknown) => void;
  }>;
  let sentCertifies: SentCommand[];

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

  function headerOf(init: RequestInit | undefined, name: string): string | null {
    const h = init?.headers;
    if (!h) return null;
    if (h instanceof Headers) return h.get(name);
    if (Array.isArray(h)) return h.find(([k]) => k.toLowerCase() === name)?.[1] ?? null;
    const rec = h as Record<string, string>;
    const k = Object.keys(rec).find((x) => x.toLowerCase() === name);
    return k === undefined ? null : rec[k]!;
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
    sentCertifies = [];

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
          const key = headerOf(init, 'idempotency-key');
          sentCertifies.push({ path, key, body });
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

  it('Switch branch, before the answer: hides the previous sheet and the Certify button immediately', async () => {
    const { el, setBranch } = mount(B01, DAY_1);
    expect(pendingReads.length).toBe(1);
    pendingReads[0]!.resolve();
    await settle();

    expect(el.querySelectorAll('[data-part]').length).toBe(6);
    expect(findCertifyButton(el)).not.toBeNull();
    expect(el.textContent).toContain(`SUMMARY-${B01}-${DAY_1}`);

    // Switch to B02; hold B02's read in flight
    setBranch(B02);
    await settle();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe(B02);

    // Before B02's read answers, no sheet and no Certify button may remain on screen
    expect(el.querySelectorAll('[data-part]').length).toBe(0);
    expect(findCertifyButton(el)).toBeNull();
  });

  it('Certify in flight, switch branch, certify answers: the new branch sheet is shown', async () => {
    const { el, setBranch } = mount(B01, DAY_1);
    pendingReads[0]!.resolve();
    await settle();

    const certifyBtn = findCertifyButton(el)!;
    expect(certifyBtn).not.toBeNull();
    certifyBtn.click();
    await settle();
    expect(pendingCertifies.length).toBe(1);

    // Switch branch to B02 while B01 certify is still in flight
    setBranch(B02);
    await settle();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe(B02);

    // B02 initial read resolves
    pendingReads[1]!.resolve();
    await settle();

    // Now B01 certify finishes; any re-read triggered by certify must read B02, not B01
    pendingCertifies[0]!.resolve();
    await settle();
    while (pendingReads.length > 2) {
      const nextRead = pendingReads.pop()!;
      nextRead.resolve();
      await settle();
    }

    expect(el.textContent).toContain(`SUMMARY-${B02}-${DAY_1}`);
    expect(el.textContent).not.toContain(`SUMMARY-${B01}-${DAY_1}`);
  });

  it('Certify sends the shown sheet branch, day, hash, note and key, and clears sheet on subsequent branch switch', async () => {
    const { el, setBranch } = mount(B01, DAY_1);
    pendingReads[0]!.resolve();
    await settle();

    const noteInput = el.querySelector<HTMLInputElement>('.reconciliation-certify input[type="text"]')!;
    noteInput.value = 'all checked';
    noteInput.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();

    findCertifyButton(el)!.click();
    await settle();
    expect(sentCertifies.length).toBe(1);
    expect(sentCertifies[0]!.body).toEqual({
      branch: B01,
      date: DAY_1,
      figuresHash: `hash-${B01}-${DAY_1}`,
      note: 'all checked',
    });
    expect(sentCertifies[0]!.key).toBeTruthy();

    pendingCertifies[0]!.resolve();
    await settle();
    pendingReads[1]!.resolve();
    await settle();

    // Switching branch immediately removes the certified sheet until the new branch responds
    setBranch(B02);
    await settle();
    expect(findCertifyButton(el)).toBeNull();
  });

  it('After a branch change, certify sends the new sheet after clearing the previous sheet during load', async () => {
    const { el, setBranch } = mount(B01, DAY_1);
    pendingReads[0]!.resolve();
    await settle();

    setBranch(B02);
    await settle();
    // While B02 is loading, B01 cannot be certified
    expect(findCertifyButton(el)).toBeNull();

    pendingReads[1]!.resolve();
    await settle();

    findCertifyButton(el)!.click();
    await settle();
    expect(sentCertifies.length).toBe(1);
    expect(sentCertifies[0]!.body).toEqual({
      branch: B02,
      date: DAY_1,
      figuresHash: `hash-${B02}-${DAY_1}`,
      note: '',
    });
  });

  it('Edge case — a day-only change behaves like a branch change (clears sheet before answer and re-reads new day after in-flight certify)', async () => {
    const { el, setDay } = mount(B01, DAY_1);
    pendingReads[0]!.resolve();
    await settle();

    expect(el.textContent).toContain(`SUMMARY-${B01}-${DAY_1}`);
    findCertifyButton(el)!.click();
    await settle();
    expect(pendingCertifies.length).toBe(1);

    // Day-only change while certify for DAY_1 is in flight
    setDay(DAY_2);
    await settle();

    // Immediately clears the sheet and Certify button before DAY_2 answers
    expect(el.querySelectorAll('[data-part]').length).toBe(0);
    expect(findCertifyButton(el)).toBeNull();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.date).toBe(DAY_2);

    // DAY_2 initial read answers
    pendingReads[1]!.resolve();
    await settle();

    // In-flight certify for DAY_1 answers; re-read must target DAY_2, not DAY_1
    pendingCertifies[0]!.resolve();
    await settle();
    while (pendingReads.length > 2) {
      const nextRead = pendingReads.pop()!;
      nextRead.resolve();
      await settle();
    }

    expect(el.textContent).toContain(`SUMMARY-${B01}-${DAY_2}`);
    expect(el.textContent).not.toContain(`SUMMARY-${B01}-${DAY_1}`);
  });
});
