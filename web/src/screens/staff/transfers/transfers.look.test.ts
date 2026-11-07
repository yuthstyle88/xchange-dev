// OL-7 row 19 (VERIFY.md on main 4a577dac, v-staff-finance-transfers.png): on /staff finance the transfers table's
// type and status cells still wrap Thai words onto two lines ("เงินเข้า/ภายนอก", "รับ/แล้ว"); the buttons no longer
// wrap (`.xc-transfers-table button`). The type (2nd) and status (3rd) columns keep their text on one line; the note
// column takes the squeeze, and `.xc-transfers-list` already scrolls sideways on a phone.
// Written before the fix by claude/studio-claude-17. Do not edit, delete or weaken.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let readHere: (name: string) => string = () => '';

function rules(css: string): Array<{ selectors: string[]; decls: Map<string, string> }> {
  const out: Array<{ selectors: string[]; decls: Map<string, string> }> = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = css.replace(/\/\*[\s\S]*?\*\//g, '');
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) decls.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    out.push({ selectors: m[1]!.split(',').map((s) => s.trim().replace(/\s+/g, ' ')), decls });
  }
  return out;
}

function nowrap(css: string, selector: string): boolean {
  return rules(css).some((r) => r.selectors.includes(selector) && r.decls.get('white-space') === 'nowrap');
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  readHere = (name) => fs.readFileSync(here + name, 'utf8');
});

describe('staff transfers table columns', () => {
  it('TT-1 the type and status columns (2nd and 3rd), head and body, do not wrap', () => {
    const css = readHere('transfers.css');
    for (const sel of [
      '.xc-transfers-table th:nth-child(2)',
      '.xc-transfers-table td:nth-child(2)',
      '.xc-transfers-table th:nth-child(3)',
      '.xc-transfers-table td:nth-child(3)',
    ]) {
      expect(nowrap(css, sel), `${sel} { white-space: nowrap }`).toBe(true);
    }
  });

  it('TT-2 guard: the 2nd and 3rd columns are still type and status, and the note column may wrap', () => {
    const src = readHere('TransfersCard.tsx');
    expect(src).toMatch(
      /<th>\{t\('col_id'\)\}<\/th>\s*<th>\{t\('col_kind'\)\}<\/th>\s*<th>\{t\('col_status'\)\}<\/th>\s*<th>\{t\('col_note'\)\}<\/th>/,
    );
    const css = readHere('transfers.css');
    expect(nowrap(css, '.xc-transfers-table td:nth-child(4)')).toBe(false);
    expect(nowrap(css, '.xc-transfers-table td')).toBe(false);
  });
});
