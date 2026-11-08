// OL-8 rates-spotlight (verification r12, "Seen outside the 138 rows"): on the /rates board the moving row spotlight
// `.xc-rb-spot` sits one header height above its row and covers the column header at load.
// Root cause, measured in Chromium with board-table.css: the spot is `position:absolute; top:0` inside
// `.xc-rb-table` (position: relative), so its frame is the table. BoardTable.place() moves it by `row.offsetTop`.
// `.xc-rb-tbody { position: relative }` makes the tbody every row's offsetParent, so `row.offsetTop` counts from the
// tbody top (first row 0), not the table top: the spot lands exactly the <thead> height too high (42 px in the probe,
// 30 px on the board). With the tbody not positioned, offsetParent is the table and the difference is 0.
// The tests pin: the spot and the rows measure from the same box, and the spotlight top equals the row top.
// Written before the fix by claude/883e8664 (pane claude-3). Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { BoardModel } from './board-model';
import { BoardTable } from './BoardTable';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'board-table.css', 'utf8');
});

function decls(selector: string): Map<string, string> | undefined {
  const plain = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(plain)) !== null) {
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

const MODEL: BoardModel = {
  headings: [{ tier: 'T1', label: '≤ 10,000 ฿' }],
  singleColumn: false,
  rows: ['USD', 'EUR', 'GBP'].map((code) => ({
    key: `${code}|ALL`,
    code,
    group: null,
    unitLabel: '',
    cells: [{ tier: 'T1', buy: '35.10', sell: '35.40', repeat: false }],
  })),
};

const HEAD = 42;
const ROW = 40;

let host: HTMLDivElement;
let sheet: HTMLStyleElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  vi.useFakeTimers();
  sheet = document.createElement('style');
  sheet.textContent = css;
  document.head.appendChild(sheet);
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  sheet.remove();
  vi.useRealTimers();
});

/**
 * jsdom has no layout. Give every body row the offsetTop a browser gives it (CSSOM offsetParent: the nearest
 * positioned ancestor): measured from the tbody when the tbody's computed position is not static, otherwise from the
 * table. In the table, row i starts at HEAD + i * ROW.
 */
function layOut(): HTMLTableSectionElement {
  const tbody = host.querySelector<HTMLTableSectionElement>('tbody')!;
  tbody.querySelectorAll<HTMLElement>('tr').forEach((row, i) => {
    Object.defineProperty(row, 'offsetTop', {
      configurable: true,
      get: () => {
        const tbodyPositioned = getComputedStyle(tbody).position !== 'static';
        return (tbodyPositioned ? 0 : HEAD) + i * ROW;
      },
    });
    Object.defineProperty(row, 'offsetHeight', { configurable: true, get: () => ROW });
  });
  return tbody;
}

describe('OL-8 rates-spotlight: the attract spotlight sits on its row', () => {
  it('RS-1 board-table.css does not position .xc-rb-tbody (rows then measure from the table, like the spot)', () => {
    expect(decls('.xc-rb-tbody')?.get('position') ?? 'static', '.xc-rb-tbody position').toBe('static');
  });

  it('RS-2 the spot is placed in the table: .xc-rb-table position relative, .xc-rb-spot absolute at top 0', () => {
    expect(decls('.xc-rb-table')?.get('position')).toBe('relative');
    expect(decls('.xc-rb-spot')?.get('position')).toBe('absolute');
    expect(decls('.xc-rb-spot')?.get('top')).toBe('0');
  });

  it('RS-3 spotlight top == row top for every row the walk lights (first row at load is below the header)', () => {
    dispose = render(() => <BoardTable model={MODEL} attract={true} />, host);
    layOut();
    const spot = host.querySelector<HTMLElement>('.xc-rb-spot')!;
    expect(spot, 'the spotlight').not.toBeNull();
    for (let i = 0; i < MODEL.rows.length; i++) {
      vi.advanceTimersByTime(1900);
      const rowTopInTable = HEAD + i * ROW;
      expect(spot.style.transform, `row ${i}: spot top == row top`).toBe(`translateY(${rowTopInTable}px)`);
      expect(spot.style.height, `row ${i}: spot height == row height`).toBe(`${ROW}px`);
    }
  });
});
