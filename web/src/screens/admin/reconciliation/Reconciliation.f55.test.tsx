// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm
// (CE, Bangkok) from the shared formatter.
// The sheet's day is a business date (→ formatBangkokDate); certifiedAt is an instant (→ formatBangkokDateTime).
// Harness copied from `Reconciliation.bangkok.test.tsx`. Written before the implementation by the ticket author. Do not
// edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
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
          createdAt: ISO_TS,
          subject: {
            reversalId: 'RV-99',
            originalTransactionId: 'TX-42',
            receiptNumber: 'RC-42',
            amountThb: '100.00',
            reason: 'reason',
            customerName: null,
            customerNo: null,
            requestedByName: 'u-teller',
            shiftId: null,
            shiftOpenedAt: null,
            lines: [],
          },
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

describe('F55: Reconciliation days and times', () => {
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

  it('F55-RC1: the certifying date reads 16/10/2026, never the raw 2026-10-16', async () => {
    const { el } = mount('2026-10-16');
    await settle();
    const head = (el.querySelector('header strong')?.textContent ?? '').trim();
    expect(head).toBe('วันที่รับรองยอด: 16/10/2026');
  });

  it('F55-RC2: in English the day reads 16/10/2026 and the certified-at stamp 16/10/2026 01:30, with no seconds', async () => {
    const { el } = mount('2026-10-16');
    await settle();
    setLang('en');
    await settle();
    const head = (el.querySelector('header strong')?.textContent ?? '').trim();
    expect(head).toBe('Certifying date: 16/10/2026');
    const certRow = el.querySelector<HTMLElement>('[data-certification="C-1"]');
    expect(certRow?.textContent).toContain('16/10/2026 01:30');
    expect(certRow?.textContent).not.toContain('01:30:00');
  });
});
