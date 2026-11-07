// OL-6 wave B staff-floor (audit #564 issuecomment-6702970101, shots 010-012): till and target balances are one
// monospace run-on line in whatever key order the answer has; the clock button is 13px/40px; the "กะของฉัน" card has an
// empty body when there is nothing to do. Written before the fix by claude/bbe9ce8b. Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeStaffFloorTarget, makeStaffFloorView, makeStaffMeView } from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { ShiftCard } from '../shift/ShiftCard';
import { en as shiftEn, th as shiftTh } from '../shift/i18n';
import { FloorCard } from './FloorCard';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

async function readHere(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8');
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function stub(): void {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if ((init?.method ?? 'GET').toUpperCase() !== 'GET') return json(200, {});
      const view = url.split('?')[0]!.replace('/api/v3/views/', '');
      if (view === 'cash_counts') return json(200, { version: 1, changed: true, data: [] });
      if (view === 'denominations') return json(200, { version: 1, changed: true, data: { rows: [] } });
      if (view === 'handovers_pending') return json(200, { version: 1, changed: true, data: [] });
      if (view === 'handover_receivers') return json(200, { version: 1, changed: true, data: { rows: [] } });
      return json(200, { version: 1, changed: true, data: null });
    }),
  );
}

function ctx(o: {
  permissions?: string[];
  myClockIn?: string | null;
  myShift?: { id: string; openedAt: string } | null;
  safe?: Record<string, string> | null;
  target?: Record<string, string> | null;
}): StaffCtx {
  return {
    me: makeStaffMeView({
      userId: '00000000-0000-4000-8000-0000000000f1',
      name: 'Ploy',
      roles: [],
      branchId: 'MAIN',
      branchName: 'XC-B',
      permissions: o.permissions ?? [],
    }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: o.myClockIn ?? null,
      myShift: o.myShift ?? null,
      cutoff: null,
      target: o.target ? makeStaffFloorTarget({ amounts: o.target }) : null,
      safe: o.safe ?? null,
    }),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

const order = (id: string) =>
  [...host.querySelectorAll<HTMLElement>(`[data-testid="${id}"] li[data-currency]`)].map((e) => e.dataset.currency);

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
  stub();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

describe('staff-floor balances', () => {
  it.each([
    [{ USD: '1', THB: '2', AUD: '3' }],
    [{ AUD: '3', USD: '1', THB: '2' }],
  ])('SF-1 till balances list THB first, then A-Z, whatever the key order (%j)', async (safe) => {
    dispose = render(() => <FloorCard ctx={ctx({ safe })} />, host);
    await settle();
    expect(order('floor-safe')).toEqual(['THB', 'AUD', 'USD']);
  });

  it('SF-2 target amounts list the same way', async () => {
    dispose = render(() => <FloorCard ctx={ctx({ target: { USD: '1', THB: '2', AUD: '3' } })} />, host);
    await settle();
    expect(order('floor-target')).toEqual(['THB', 'AUD', 'USD']);
  });

  it('SF-3 a row shows the code and the money amount apart', async () => {
    dispose = render(() => <FloorCard ctx={ctx({ safe: { USD: '1000' } })} />, host);
    await settle();
    const row = host.querySelector('[data-testid="floor-safe"] li[data-currency="USD"]');
    expect(row?.querySelector('.xc-floor-balance-code')?.textContent).toBe('USD');
    expect(row?.querySelector('.xc-floor-balance-amount')?.textContent).toBe('1,000.00');
    expect(host.querySelector('[data-testid="floor-safe"]')?.tagName).toBe('UL');
    expect(host.querySelector('[data-testid="floor-safe"]')?.classList.contains('xc-floor-balances')).toBe(true);
  });

  it('SF-4 the list and its rows are styled, with tabular sans figures and no monospace', async () => {
    const css = await readHere('floor.css');
    const list = decls(css, '.xc-floor-balances');
    expect(list?.get('list-style')).toBe('none');
    expect(list?.get('padding')).toBe('0');
    expect(list?.get('display')).toBe('grid');
    const row = decls(css, '.xc-floor-balance');
    expect(row?.get('display')).toBe('flex');
    expect(row?.get('justify-content')).toBe('space-between');
    expect(decls(css, '.xc-floor-balance-amount')?.get('font-variant-numeric')).toBe('tabular-nums');
    expect(css).not.toContain('font-mono');
  });
});

describe('staff-floor controls', () => {
  it('SF-5 the floor buttons, clock button included, have the 44px shape', async () => {
    const d = decls(await readHere('floor.css'), '.xc-floor-clock-btn');
    expect(d?.get('min-height')).toBe('44px');
    expect(d?.get('padding')).toBe('10px 18px');
    expect(d?.get('font-size')).toBe('15px');
    expect(d?.get('font-weight')).toBe('600');
    expect(d?.get('border-radius')).toBe('var(--xc-radius-md)');
    expect(d?.get('color')).toBe('var(--xc-brand)');
  });

  it('SF-6 the target input leaves height, radius and border to the theme', async () => {
    const d = decls(await readHere('floor.css'), '.xc-floor-target-input');
    for (const prop of ['padding', 'border', 'border-radius', 'font-size']) {
      expect(d?.has(prop) ?? false, `.xc-floor-target-input must not set ${prop}`).toBe(false);
    }
  });

  it('SF-7 shift figures are tabular sans; the action button is 44px; count inputs and the select use the theme', async () => {
    const css = await readHere('../shift/shift.css');
    expect(css).not.toContain('font-mono');
    expect(decls(css, '.xc-shift-denom-val')?.get('font-variant-numeric')).toBe('tabular-nums');
    const btn = decls(css, '.xc-shift-action-btn');
    expect(btn?.get('min-height')).toBe('44px');
    expect(btn?.get('padding')).toBe('10px 18px');
    expect(btn?.get('border-radius')).toBe('var(--xc-radius-md)');
    expect(btn?.get('color')).toBe('var(--xc-brand)');
    const input = decls(css, '.xc-shift-denom-input');
    expect(input?.get('width')).toBe('6rem');
    expect(input?.get('text-align')).toBe('right');
    for (const prop of ['padding', 'border', 'border-radius', 'font-size']) {
      expect(input?.has(prop), `.xc-shift-denom-input must not set ${prop}`).toBe(false);
      expect(decls(css, '.xc-shift-select')?.has(prop) ?? false, `.xc-shift-select must not set ${prop}`).toBe(false);
    }
  });
});

describe('staff-floor shift card empty state', () => {
  it('SF-8 an operator who has not clocked in is told to clock in first', async () => {
    dispose = render(() => <ShiftCard ctx={ctx({ permissions: ['ShiftOperate'] })} />, host);
    await settle();
    expect(host.querySelector('[data-testid="shift-empty"]')?.textContent).toBe('ลงเวลาเข้างานก่อน จึงจะเปิดกะได้');
  });

  it('SF-9 a certifier with nothing to verify sees that nothing is waiting', async () => {
    dispose = render(() => <ShiftCard ctx={ctx({ permissions: ['ShiftCloseCertify'] })} />, host);
    await settle();
    expect(host.querySelector('[data-testid="shift-empty"]')?.textContent).toBe('ไม่มีรายการที่ต้องทำในตอนนี้');
  });

  it('SF-10 a clocked-in operator sees the actions and no empty line', async () => {
    const c = ctx({ permissions: ['ShiftOperate'], myClockIn: '2026-10-11T01:00:00Z' });
    dispose = render(() => <ShiftCard ctx={c} />, host);
    await settle();
    expect(host.querySelector('[data-testid="shift-open-count"]')).not.toBeNull();
    expect(host.querySelector('[data-testid="shift-empty"]')).toBeNull();
  });

  it('SF-11 the texts are exact in both languages and the line is muted', async () => {
    expect(shiftTh.emptyClockIn).toBe('ลงเวลาเข้างานก่อน จึงจะเปิดกะได้');
    expect(shiftEn.emptyClockIn).toBe('Clock in first to open a shift.');
    expect(shiftTh.emptyNothing).toBe('ไม่มีรายการที่ต้องทำในตอนนี้');
    expect(shiftEn.emptyNothing).toBe('Nothing to do right now.');
    const d = decls(await readHere('../shift/shift.css'), '.xc-shift-empty');
    expect(d?.get('margin')).toBe('0');
    expect(d?.get('color')).toBe('var(--xc-muted)');
  });
});
