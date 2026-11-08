// OL-7 (c) (verification ~/xc-verify-138, "Seen outside the 138 rows"): /kiosk-currencies.html
//   1. draws in an Arial-like face: `.xc-kiosk { font-family: Arial, Tahoma, … }` (kiosk-currencies.css:14) names
//      Arial first, so Latin text and digits are Arial and Thai falls to Tahoma, not the theme's Noto Sans Thai;
//   2. at phone width each card's KB table cuts its "เวลาประกาศ" column at the card edge: every th/td is
//      `white-space: nowrap`, and the stamp ("7/11/2569 20:57:41 (เวลาไทย) · รอบ 10") makes the table 502 px in a
//      364 px box at 430 px (measured on feat/one-look-7 06b1e00b with a 4-currency fixture).
// Fix: the theme face token, and at ≤650px the stamp column wraps (headers stay on one line), 13px text, 4px cells.
// Measured with that fix: the table fits its box at 375 and 430; at 320 the one-line headers alone are 286 px, so the
// box still scrolls there (accepted; headers are pinned by KioskCurrenciesScreen.test.tsx:454).
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const norm = (s: string) => s.replace(/\s+/g, '');
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const block = (body: string, media: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const k = d.indexOf(':');
        if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
      }
      for (const sel of m[1]!.split(',')) out.push({ media, selector: normSel(sel), decls });
    }
  };
  let rest = '';
  let i = 0;
  while (i < src.length) {
    const at = src.indexOf('@media', i);
    if (at < 0) {
      rest += src.slice(i);
      break;
    }
    rest += src.slice(i, at);
    const open = src.indexOf('{', at);
    let depth = 1;
    let j = open + 1;
    while (j < src.length && depth > 0) {
      if (src[j] === '{') depth += 1;
      if (src[j] === '}') depth -= 1;
      j += 1;
    }
    block(src.slice(open + 1, j - 1), norm(src.slice(at + 6, open)));
    i = j;
  }
  block(rest, '');
  return out;
}

let raw = '';
let rules: Rule[] = [];

function value(selector: string, prop: string, media = ''): string | undefined {
  const m = media.replace(/\s+/g, '');
  for (let i = rules.length - 1; i >= 0; i -= 1) {
    const r = rules[i]!;
    if (r.selector === selector && r.media === m && r.decls.has(prop)) return r.decls.get(prop);
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  raw = fs.readFileSync(here + 'kiosk-currencies.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  rules = parseCss(raw);
});

const PHONE = '(max-width:650px)';
const KB = '.xc-kiosk .xc-shell-main .kbank-box';

describe('KF kiosk-currencies face and KB table (OL-7 c)', () => {
  it('KF-1 the page uses the theme face token', () => {
    expect(value('.xc-kiosk', 'font-family')).toBe('var(--xc-font-sans)');
  });

  it('KF-2 no rule in the stylesheet names Arial or Tahoma', () => {
    expect(raw).not.toMatch(/\b(Arial|Tahoma)\b/);
  });

  it('KF-3 at ≤650px the KB stamp column wraps; the table drops to 13px with 4px cells (fits a 375 px phone)', () => {
    expect(value(`${KB} td:last-child`, 'white-space', PHONE)).toBe('normal');
    expect(value(`${KB} table`, 'font-size', PHONE)).toBe('13px');
    expect(value(`${KB} th`, 'padding', PHONE)).toBe('4px');
    expect(value(`${KB} td`, 'padding', PHONE)).toBe('4px');
  });

  it('KF-4 headers and the other cells stay on one line', () => {
    expect(value(`${KB} th`, 'white-space')).toBe('nowrap');
    expect(value(`${KB} td`, 'white-space')).toBe('nowrap');
    expect(value(`${KB} th`, 'white-space', PHONE)).toBeUndefined();
  });
});
