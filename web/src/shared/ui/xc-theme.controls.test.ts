// OL-6 A2 (audit #564 issuecomment-6702970101, R3 + R7): staff native controls get the theme, and a disabled button keeps
// readable contrast. Today the theme leaves checkbox, radio, the file-input button and <summary> to the browser, and a
// disabled button is `opacity: 0.5` (pink on pink, below 4.5:1). --xc-gray-600 on --xc-gray-100 is 4.99:1.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { selector: string; decls: Map<string, string> };

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
    out.push({ selector: m[1]!.replace(/\s+/g, ' ').trim(), decls });
  }
  return out;
}

/** The rules whose selector list matches `test`, merged in source order (later wins). */
function merged(all: Rule[], test: (sel: string) => boolean): Map<string, string> {
  const out = new Map<string, string>();
  for (const r of all) if (test(r.selector)) for (const [k, v] of r.decls) out.set(k, v);
  return out;
}

/** True when `sel` names `needle` outside any :not(...). */
function namesOutsideNot(sel: string, needle: string): boolean {
  return sel.replace(/:not\([^()]*\)/g, '').includes(needle);
}

let all: Rule[] = [];

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  all = rules(fs.readFileSync(here + 'xc-theme.css', 'utf8'));
});

describe('A2 staff controls in xc-theme.css', () => {
  it('A2-1 a disabled button has solid colours and no opacity', () => {
    const d = merged(all, (s) => s.includes(':disabled'));
    expect(d.size, 'a :disabled rule exists').toBeGreaterThan(0);
    expect(d.has('opacity'), 'no opacity on disabled').toBe(false);
    expect(d.get('background')).toBe('var(--xc-gray-100)');
    expect(d.get('border-color')).toBe('var(--xc-line)');
    expect(d.get('color')).toBe('var(--xc-gray-600)');
    expect(d.get('box-shadow')).toBe('none');
    expect(d.get('cursor')).toBe('not-allowed');
  });

  it('A2-2 checkbox and radio take the brand accent at 20 x 20 px', () => {
    for (const type of ['checkbox', 'radio']) {
      const d = merged(all, (s) => namesOutsideNot(s, `input[type="${type}"]`));
      expect(d.get('accent-color'), type).toBe('var(--xc-brand)');
      expect(d.get('width'), type).toBe('20px');
      expect(d.get('height'), type).toBe('20px');
    }
  });

  it('A2-3 the file-input button looks like the secondary button', () => {
    const d = merged(all, (s) => s.includes('::file-selector-button'));
    expect(d.get('border')).toBe('1px solid var(--xc-line)');
    expect(d.get('border-radius')).toBe('var(--xc-radius-md)');
    expect(d.get('background')).toBe('var(--xc-surface)');
    expect(d.get('color')).toBe('var(--xc-brand)');
  });

  it('A2-4 <summary> reads as a control', () => {
    const d = merged(all, (s) => /(^|[\s(,>])summary(?![\w-])/.test(s.replace(/:not\([^()]*\)/g, '')));
    expect(d.get('cursor')).toBe('pointer');
    expect(d.get('font-weight')).toBe('600');
  });
});
