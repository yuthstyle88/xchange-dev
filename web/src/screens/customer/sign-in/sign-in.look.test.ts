// OL-6 wave B (cust-sign-in, audit #564 issuecomment-6702970101): title gap, channel note, marketing checkbox and password label gap.
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
  rules = parseCss(fs.readFileSync(here + 'sign-in.css', 'utf8'));
});

const CHECK = '.xc-sign-in-form label.xc-sign-in-check';

describe('cust-sign-in', () => {
  it('SI-1 the title keeps a 12px gap above the tabs', () => {
    expect(value('.xc-sign-in>h2', 'margin')).toBe('0 0 12px');
  });

  it('SI-2 the default note takes its own row under the channel buttons', () => {
    expect(value('.xc-sign-in-mode', 'flex-wrap')).toBe('wrap');
    expect(value('.xc-sign-in-mode>[data-default-note]', 'flex')).toBe('0 0 100%');
  });

  it('SI-3 the marketing checkbox sits beside its label: the rule outranks .xc-sign-in-form label', () => {
    expect(value(CHECK, 'flex-direction')).toBe('row');
    expect(value(CHECK, 'align-items')).toBe('center');
    expect(value(CHECK, 'gap')).toBe('8px');
    expect(value(CHECK, 'min-height')).toBe('44px');
    expect(rules.some((r) => r.selector === '.xc-sign-in-check')).toBe(false);
  });

  it('SI-4 the marketing checkbox is 20px and does not shrink', () => {
    expect(value(`${CHECK} input`, 'width')).toBe('20px');
    expect(value(`${CHECK} input`, 'height')).toBe('20px');
    expect(value(`${CHECK} input`, 'flex')).toBe('none');
  });

  it('SI-5 the password field keeps the same label gap as the other fields', () => {
    expect(value('.xc-sign-in-form .xc-field', 'display')).toBe('flex');
    expect(value('.xc-sign-in-form .xc-field', 'flex-direction')).toBe('column');
    expect(value('.xc-sign-in-form .xc-field', 'gap')).toBe('0.25rem');
  });
});
