// OL-6 A3 (audit #564 issuecomment-6702970101, R4): the admin base rules stop fighting the theme.
// Today admin.css sets Tahoma, gives every button a transparent border (secondaries vanish on the grey page), dims
// disabled buttons with opacity, sizes radios like text fields (~40 px circles), puts every checkbox on its own line
// above its text, leaves <textarea> unstyled, and letter-spaces Thai eyebrows.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { selector: string; decls: Map<string, string> };

/** Splits a selector list on top-level commas only (not inside :has(...) / :is(...)). */
function splitSelectors(text: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = '';
  for (const ch of text) {
    if (ch === '(') depth++;
    else if (ch === ')') depth--;
    if (ch === ',' && depth === 0) {
      out.push(cur);
      cur = '';
    } else cur += ch;
  }
  if (cur.trim()) out.push(cur);
  return out;
}

function rules(css: string): Rule[] {
  const src = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const k = d.indexOf(':');
      if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
    }
    for (const sel of splitSelectors(m[1]!)) out.push({ selector: sel.replace(/\s+/g, ' ').trim(), decls });
  }
  return out;
}

const P = '.xc-admin .xc-content.xc-content';
let all: Rule[] = [];
const exact = (sel: string) => all.filter((r) => r.selector === sel);
const last = (sel: string, prop: string) => exact(sel).map((r) => r.decls.get(prop)).filter((v) => v !== undefined).pop();

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  all = rules(fs.readFileSync(here + 'admin.css', 'utf8'));
});

describe('A3 admin base rules', () => {
  it('A3-1 .xc-admin uses the theme font, not Tahoma', () => {
    const font = last('.xc-admin', 'font') ?? '';
    expect(font).toContain('var(--xc-font-sans)');
    expect(font).not.toMatch(/Tahoma/i);
  });

  it('A3-2 the base admin button border is 1px solid var(--xc-line)', () => {
    expect(last(`${P} button`, 'border')).toBe('1px solid var(--xc-line)');
  });

  it('A3-3 a disabled admin button has solid colours and no opacity', () => {
    const sel = `${P} button:disabled`;
    expect(exact(sel).some((r) => r.decls.has('opacity')), 'no opacity').toBe(false);
    expect(last(sel, 'background')).toBe('var(--xc-gray-100)');
    expect(last(sel, 'border-color')).toBe('var(--xc-line)');
    expect(last(sel, 'color')).toBe('var(--xc-gray-600)');
  });

  it('A3-4 the text-field rule excludes radios and includes textarea', () => {
    expect(exact(`${P} input:not([type="checkbox"])`), 'the old selector that caught radios is gone').toEqual([]);
    expect(last(`${P} input:not([type="checkbox"]):not([type="radio"])`, 'width')).toBe('100%');
    expect(last(`${P} textarea`, 'width')).toBe('100%');
    expect(last(`${P} textarea`, 'border')).toBe('1px solid var(--xc-line)');
    expect(last(`${P} textarea`, 'min-height')).toBe('88px');
  });

  it('A3-5 a label holding a checkbox or radio lays it out inline, beside its text', () => {
    const sel = `${P} label:not(.check-field):has(> input[type="checkbox"], > input[type="radio"])`;
    expect(last(sel, 'display'), sel).toBe('flex');
    expect(last(sel, 'align-items')).toBe('center');
    const input = `${P} label:not(.check-field) input:is([type="checkbox"], [type="radio"])`;
    expect(last(input, 'display'), input).toBe('inline-block');
    expect(last(input, 'margin-top')).toBe('0');
  });

  it('A3-6 a Thai eyebrow has no letter-spacing', () => {
    expect(last(`${P} .eyebrow:lang(th)`, 'letter-spacing')).toBe('0');
  });
});
