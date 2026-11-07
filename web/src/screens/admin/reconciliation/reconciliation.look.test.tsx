// OL-6 wave B admin (audit #564 issuecomment-6702970101, row 030): the reconciliation sheet's look. Parts are the shared
// white `.card`, tables scroll inside their card with money right-aligned, empty parts say so, lists lose the browser
// bullets, the certify block is laid out with a primary button, and a raw UUID shows in a short form because the sheet
// carries no name (generated/finance_reconciliation.ts:7,14). Written before the fix by claude/94389a6b. Do not edit,
// delete or weaken; add your own elsewhere.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Reconciliation } from './Reconciliation';
import { th } from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

function decls(selector: string): Map<string, string> | undefined {
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

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'reconciliation.css', 'utf8');
});

const UUID = '50587ce9-5c11-456b-afda-23ca4653e43c';
const on = <T,>(data: T) => ({ available: true, reason: null, data });

function sheet(over: Record<string, unknown> = {}) {
  return {
    branch: 'MAIN',
    date: '2026-11-07',
    transactions: on({
      byMethod: [{ method: 'Cash', buyThb: '1300.00', sellThb: '2000.00', fees: '30.00', bankFees: '0.00', count: 3 }],
      reversedThb: '250.00',
    }),
    counts: on([
      {
        countId: 'c1',
        kind: 'CLOSE',
        countedBy: UUID,
        verifiedBy: UUID,
        lines: [{ currency: 'USD', counted: '10', expected: '10', difference: '0' }],
      },
    ]),
    transfers: on({ byStatus: [{ status: 'SENT', count: 1 }], inTransit: [], shortReceipts: [] }),
    exceptions: on([{ id: 'EX-1', position: 1, reviewed: false, summary: 'short count' }]),
    integrity: on([{ check: 1, ok: true, offenders: 0 }]),
    reversalsPending: on([
      {
        requestId: 'r1',
        createdAt: '2026-11-07T01:00:00Z',
        subject: { receiptNumber: 'R-1', amountThb: '100.00', reason: 'x', requestedByName: 'A', customerName: 'B', shiftOpenedAt: null },
      },
    ]),
    figuresHash: 'h',
    certifications: [{ id: 'C-1', certifiedBy: UUID, certifiedAt: '2026-11-07T02:00:00Z', note: null, figuresHash: 'h', stale: false }],
    ...over,
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

async function mount(data: unknown): Promise<void> {
  vi.stubGlobal(
    'fetch',
    vi.fn(
      async () =>
        new Response(JSON.stringify({ version: 1, changed: true, data }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    ),
  );
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-11-07', to: '2026-11-07' });
  const [shown] = createSignal(true);
  dispose = render(() => <Reconciliation branch={branch} period={period} shown={shown} />, host);
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
  expect(host.querySelector('[data-part="transactions"]'), 'the sheet rendered').not.toBeNull();
}

const part = (name: string) => host.querySelector(`[data-part="${name}"]`)!;

describe('R1 white cards, not grey boxes', () => {
  it('R1-1 every part and the certify block carry the shared card class', async () => {
    await mount(sheet());
    const boxes = host.querySelectorAll('.reconciliation-part, .reconciliation-certify');
    expect(boxes.length).toBe(7);
    for (const box of boxes) expect(box.classList.contains('card'), box.className).toBe(true);
  });

  it('R1-2 the screen CSS copies no card values', () => {
    expect(decls('.reconciliation-part,\n.reconciliation-certify')).toBeUndefined();
    expect(css).not.toMatch(/border-radius/);
  });

  it('R1-3 the column cannot grow past the page and the grid gap is the only space between cards', () => {
    expect(decls('.reconciliation')?.get('grid-template-columns')).toBe('minmax(0, 1fr)');
    expect(decls('.xc-admin .xc-content .reconciliation .card')?.get('margin-bottom')).toBe('0');
  });

  it('R1-4 part headings have one size and one gap', () => {
    const h = decls('.xc-admin .xc-content .reconciliation .card h3');
    expect(h?.get('margin')).toBe('0 0 12px');
    expect(h?.get('font-size')).toBe('16px');
  });
});

describe('R2 the transactions table', () => {
  it('R2-1 scrolls inside its card', async () => {
    await mount(sheet());
    expect(part('transactions').querySelector('.table-scroll table.reconciliation-table')).not.toBeNull();
  });

  it('R2-2 the five money and count columns use the admin .num, the method column does not', async () => {
    await mount(sheet());
    const heads = [...part('transactions').querySelectorAll('th')];
    const cells = [...part('transactions').querySelectorAll('tbody td')];
    expect(heads.map((c) => c.classList.contains('num'))).toEqual([false, true, true, true, true, true]);
    expect(cells.map((c) => c.classList.contains('num'))).toEqual([false, true, true, true, true, true]);
  });

  it('R2-3 the rules the admin table rule always beat are gone', () => {
    expect(decls('.reconciliation-table th,\n.reconciliation-table td')).toBeUndefined();
    expect(decls('.reconciliation-table th:first-child,\n.reconciliation-table td:first-child')).toBeUndefined();
  });

  it('R2-4 the reversed total has room under the table', async () => {
    await mount(sheet());
    const p = part('transactions').querySelector('.reconciliation-reversed');
    expect(p?.textContent).toContain('250.00');
    expect(decls('.xc-admin .xc-content .reconciliation-reversed')?.get('margin-top')).toBe('12px');
  });
});

describe('R3 empty parts say so', () => {
  it('R3-1 no exchanges: "none" instead of a bare header row; the reversed total still shows', async () => {
    await mount(sheet({ transactions: on({ byMethod: [], reversedThb: '0.00' }) }));
    expect(part('transactions').querySelector('table')).toBeNull();
    expect(part('transactions').querySelector('.reconciliation-none')?.textContent).toBe(th.none);
    expect(part('transactions').querySelector('.reconciliation-reversed')?.textContent).toContain('0.00');
  });

  it('R3-2 no transfers: "none" instead of an empty box', async () => {
    await mount(sheet({ transfers: on({ byStatus: [], inTransit: [], shortReceipts: [] }) }));
    expect(part('transfers').querySelector('.reconciliation-none')?.textContent).toBe(th.none);
    expect(part('transfers').querySelector('ul')).toBeNull();
  });

  it('R3-3 a transfer row keeps the list and shows no "none"', async () => {
    await mount(sheet());
    expect(part('transfers').querySelector('.reconciliation-none')).toBeNull();
    expect(part('transfers').querySelector('ul li')?.textContent).toContain('SENT');
  });
});

describe('R4 lists and the certify block', () => {
  it('R4-1 lists have no browser bullets or indent', () => {
    const d = decls('.xc-admin .xc-content .reconciliation ul');
    expect(d?.get('list-style')).toBe('none');
    expect(d?.get('margin')).toBe('0');
    expect(d?.get('padding')).toBe('0');
    expect(decls('.xc-admin .xc-content .reconciliation li + li')?.get('margin-top')).toBe('8px');
  });

  it('R4-2 certify is the primary button', async () => {
    await mount(sheet());
    const certify = [...host.querySelectorAll<HTMLButtonElement>('.reconciliation-certify button')].find(
      (b) => b.textContent === th.certify,
    );
    expect(certify?.classList.contains('primary')).toBe(true);
  });

  it('R4-3 the certify block is a grid with one gap and its button does not stretch', () => {
    const d = decls('.xc-admin .xc-content .reconciliation-certify');
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('gap')).toBe('12px');
    expect(decls('.xc-admin .xc-content .reconciliation-certify > button')?.get('justify-self')).toBe('start');
  });

  it('R4-4 both character counters are small muted right-aligned text', async () => {
    await mount(sheet());
    const counters = [...host.querySelectorAll('.reconciliation-counter')].map((c) => c.textContent);
    expect(counters).toEqual(['0 / 500', '0 / 500']);
    const d = decls('.reconciliation-counter');
    expect(d?.get('font-size')).toBe('12px');
    expect(d?.get('color')).toBe('var(--xc-muted)');
    expect(d?.get('text-align')).toBe('right');
  });

  it('R4-5 a pending reversal is one row of summary, reason, counter and two buttons', async () => {
    await mount(sheet());
    expect(part('reversalsPending').querySelector('li[data-request].reconciliation-reversal')).not.toBeNull();
    const d = decls('.xc-admin .xc-content .reconciliation-reversal');
    expect(d?.get('display')).toBe('flex');
    expect(d?.get('flex-wrap')).toBe('wrap');
    expect(d?.get('gap')).toBe('8px');
    const whole = decls(
      '.xc-admin .xc-content .reconciliation-reversal > [data-summary],\n.xc-admin .xc-content .reconciliation-reversal > input,\n.xc-admin .xc-content .reconciliation-reversal > .reconciliation-counter',
    );
    expect(whole?.get('flex-basis')).toBe('100%');
  });
});

describe('R5 raw ids', () => {
  it('R5-1 a certifier id that is a UUID shows its first 8 characters, the whole id on hover', async () => {
    await mount(sheet());
    const row = host.querySelector('[data-certification="C-1"]')!;
    expect(row.textContent).toContain('50587ce9');
    expect(row.textContent).not.toContain('-5c11-');
    expect(row.querySelector(`[title="${UUID}"]`)?.textContent).toBe('50587ce9');
  });

  it('R5-2 a counter and verifier id that is a UUID shows the same way', async () => {
    await mount(sheet());
    const text = part('counts').textContent ?? '';
    expect(text).toContain('50587ce9');
    expect(text).not.toContain('-5c11-');
  });

  it('R5-3 an id that is not a UUID is shown as sent', async () => {
    const row = { id: 'C-1', certifiedBy: 'u-fin', certifiedAt: '2026-11-07T02:00:00Z', note: null, figuresHash: 'h', stale: false };
    await mount(sheet({ certifications: [row] }));
    expect(host.querySelector('[data-certification="C-1"]')?.textContent).toContain('u-fin');
  });
});
