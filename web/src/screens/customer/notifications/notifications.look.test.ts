// OL-6 wave B (cust-notifications, audit #564 issuecomment-6702970101): gutter, filter touch target, empty card, disabled buttons.
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
  rules = parseCss(fs.readFileSync(here + 'Notifications.css', 'utf8'));
});

describe('cust-notifications', () => {
  it('NO-1 the screen gutter is 1rem', () => {
    expect(value('.xc-notifications-screen', 'padding')).toBe('1rem 1rem 2.5rem');
  });

  it('NO-2 the filter is a 44px target with a 20px checkbox', () => {
    expect(value('.xc-notifications-filter', 'min-height')).toBe('44px');
    expect(value('.xc-notifications-filter-input', 'width')).toBe('20px');
    expect(value('.xc-notifications-filter-input', 'height')).toBe('20px');
    expect(value('.xc-notifications-filter-input', 'flex')).toBe('none');
  });

  it('NO-3 the toolbar wraps instead of squeezing the filter', () => {
    expect(value('.xc-notifications-toolbar', 'flex-wrap')).toBe('wrap');
  });

  it('NO-4 the empty state is a card', () => {
    expect(value('.xc-notifications-empty', 'background')).toBe('var(--xc-card)');
    expect(value('.xc-notifications-empty', 'border-radius')).toBe('var(--xc-radius-card)');
    expect(value('.xc-notifications-empty', 'box-shadow')).toBe('var(--xc-shadow-card)');
  });

  it('NO-5 disabled buttons are solid and readable', () => {
    for (const sel of ['.xc-notifications-mark-all-btn:disabled', '.xc-notifications-mark-btn:disabled']) {
      expect(value(sel, 'opacity'), sel).toBeUndefined();
      expect(value(sel, 'background'), sel).toBe('var(--xc-gray-100)');
      expect(value(sel, 'color'), sel).toBe('var(--xc-gray-600)');
      expect(value(sel, 'border-color'), sel).toBe('var(--xc-border)');
    }
  });
});
