// #2498: Admin Reconciliation — fallback day when `period().to` is empty must use Asia/Bangkok's today
// (not the browser's local timezone), and `certifiedAt` / `reversalsPending.createdAt` must be formatted
// in Asia/Bangkok via `new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' })`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { locale, setLang } from '../../../shared/i18n';
import { Reconciliation } from './Reconciliation';

const VIEW = '/api/v3/views/finance_reconciliation';
const B01 = 'XC-B01';
const ISO_TS = '2026-10-15T18:30:00+00:00'; // 2026-10-16 01:30 in Asia/Bangkok (+07:00)

function waiting(reason: string) {
  return { available: false, reason, data: null };
}

function makeSheet(branch: string, date: string) {
  return {
    branch,
    date,
    transactions: {
      available: true,
      reason: null,
      data: {
        byMethod: [],
        reversedThb: '0.00',
      },
    },
    counts: waiting('CA-3'),
    transfers: waiting('CA-5'),
    exceptions: {
      available: true,
      reason: null,
      data: [],
    },
    integrity: waiting('LG-7'),
    reversalsPending: {
      available: true,
      reason: null,
      data: [
        {
          requestId: 'RV-99',
          transactionId: 'TX-42',
          requestedBy: 'u-teller',
          createdAt: ISO_TS,
        },
      ],
    },
    figuresHash: `hash-${branch}-${date}`,
    certifications: [
      {
        id: 'C-1',
        certifiedBy: 'u-fin',
        certifiedAt: ISO_TS,
        note: 'checked',
        figuresHash: `hash-${branch}-${date}`,
        stale: false,
      },
    ],
  };
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
  }
}

describe('Reconciliation Asia/Bangkok today and timestamps (#2498)', () => {
  const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env;
  const originalTz = env.TZ;
  let root: HTMLElement | undefined;
  let dispose: (() => void) | undefined;
  let readDates: string[];

  beforeEach(() => {
    env.TZ = 'UTC';
    vi.useFakeTimers();
    // 2026-10-15T23:30:00Z is still 15 Oct in UTC, but 06:30 on 16 Oct in Asia/Bangkok
    vi.setSystemTime(new Date('2026-10-15T23:30:00Z'));
    setLang('th');
    readDates = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (url.pathname === VIEW && method === 'GET') {
          const branch = url.searchParams.get('branch') ?? B01;
          const date = url.searchParams.get('date') ?? '';
          readDates.push(date);
          return jsonResponse(200, {
            version: 1,
            changed: true,
            data: makeSheet(branch, date),
          });
        }
        return jsonResponse(200, { version: 1, changed: false });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    document.body.innerHTML = '';
    vi.useRealTimers();
    vi.unstubAllGlobals();
    setLang('th');
    if (originalTz === undefined) {
      delete env.TZ;
    } else {
      env.TZ = originalTz;
    }
  });

  function mount(initialTo: string) {
    const [branch] = createSignal(B01);
    const [period, setPeriod] = createSignal({ from: '2026-10-01', to: initialTo });
    const [shown] = createSignal(true);
    const el = document.createElement('div');
    root = el;
    document.body.appendChild(el);
    dispose = render(
      () => (<Reconciliation branch={branch} period={period} shown={shown} />) as JSX.Element,
      el,
    );
    return { el, setPeriod };
  }

  it('row 1: Empty To date with device clock at 2026-10-15T23:30:00Z (06:30 Bangkok on the 16th) reads the sheet for 2026-10-16', async () => {
    mount('');
    await settle();
    expect(readDates).toEqual(['2026-10-16']);
  });

  it('row 2: To date 2026-10-10 reads 2026-10-10 unchanged, and clearing To falls back to 2026-10-16 in Bangkok', async () => {
    const { setPeriod } = mount('2026-10-10');
    await settle();
    expect(readDates).toEqual(['2026-10-10']);

    setPeriod({ from: '2026-10-01', to: '' });
    await settle();
    expect(readDates).toEqual(['2026-10-10', '2026-10-16']);
  });

  it('row 3: certifiedAt 2026-10-15T18:30:00+00:00 is shown as 16 Oct 01:30 in Bangkok time (never raw UTC)', async () => {
    const { el } = mount('2026-10-16');
    await settle();

    const certRow = el.querySelector<HTMLElement>('[data-certification="C-1"]');
    expect(certRow).not.toBeNull();
    const expectedTh = new Date(ISO_TS).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
    expect(certRow!.textContent).toContain(expectedTh);
    expect(certRow!.textContent).not.toContain(ISO_TS);

    setLang('en');
    await settle();
    const expectedEn = new Date(ISO_TS).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
    expect(certRow!.textContent).toContain(expectedEn);
    expect(certRow!.textContent).toMatch(/16/);
    expect(certRow!.textContent).toMatch(/1:30/);
    expect(certRow!.textContent).not.toContain(ISO_TS);
  });

  it('edge: reversal request createdAt is shown as Bangkok date + time (never raw UTC)', async () => {
    const { el } = mount('2026-10-16');
    await settle();

    const pendingPart = el.querySelector<HTMLElement>('[data-part="reversalsPending"]');
    expect(pendingPart).not.toBeNull();
    const expectedTh = new Date(ISO_TS).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
    expect(pendingPart!.textContent).toContain('RV-99');
    expect(pendingPart!.textContent).toContain(expectedTh);
    expect(pendingPart!.textContent).not.toContain(ISO_TS);
  });
});
