// OL-6 wave B (cust-verification, audit #564 issuecomment-6702970101): stepper, primary action, disabled buttons, card surface.
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

let rules: Rule[] = [];

/** The value the stylesheet's last rule for this exact selector gives the property, inside `media` when named. */
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
  rules = parseCss(fs.readFileSync(here + 'verification.css', 'utf8'));
});

const PRIMARY = ['start', 'submit', 'resubmit', 'update-and-resubmit'].map(
  (a) => `.xc-verification-btn[data-action="${a}"]`,
);

describe('cust-verification', () => {
  it('VE-1 each step starts at the same line under a progress bar', () => {
    expect(value('.xc-verification-step', 'justify-content')).toBe('flex-start');
    expect(value('.xc-verification-step', 'border-top')).toBe('3px solid var(--xc-border)');
    expect(value('.xc-verification-step', 'border-radius')).toBe('0');
    expect(value('.xc-verification-step', 'padding')).toBe('0.5rem 0.25rem 0');
  });

  it('VE-2 the active step is marked in colour as well as weight', () => {
    expect(value('.xc-verification-step--active', 'border-top-color')).toBe('var(--xc-brand)');
    expect(value('.xc-verification-step--active', 'color')).toBe('var(--xc-brand)');
    expect(value('.xc-verification-step--active', 'font-weight')).toBe('700');
  });

  it('VE-3 the main action of each state is the filled primary', () => {
    for (const sel of PRIMARY) {
      expect(value(sel, 'background'), sel).toBe('var(--xc-brand)');
      expect(value(sel, 'border-color'), sel).toBe('var(--xc-brand)');
      expect(value(sel, 'color'), sel).toBe('var(--xc-on-brand)');
    }
  });

  it('VE-4 disabled is solid, and its rule comes after the primary rule that has the same specificity', () => {
    const DISABLED = '.xc-verification-btn:disabled';
    expect(value(DISABLED, 'opacity')).toBeUndefined();
    expect(value(DISABLED, 'background')).toBe('var(--xc-gray-100)');
    expect(value(DISABLED, 'border-color')).toBe('var(--xc-border)');
    expect(value(DISABLED, 'color')).toBe('var(--xc-gray-600)');
    const at = (s: string) => rules.findIndex((r) => r.selector === s);
    for (const sel of PRIMARY) expect(at(DISABLED), sel).toBeGreaterThan(at(sel));
  });

  it('VE-5 a section is a customer card', () => {
    expect(value('.xc-verification-card', 'background')).toBe('var(--xc-card)');
    expect(value('.xc-verification-card', 'border-radius')).toBe('var(--xc-radius-card)');
    expect(value('.xc-verification-card', 'box-shadow')).toBe('var(--xc-shadow-card)');
  });
});
