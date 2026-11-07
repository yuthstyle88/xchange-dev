// OL-6 B card-phone: the shared `.xc-theme .card` kept 20px padding at phone width, while the mock-up's card pads 16px
// at phone width (docs/design/one-look-mock/index.html:68-73 says 760px; the OL-6 order and L7b use 800px). Measured at 375
// before this ticket: padding 20px.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let readHere: (name: string) => string = () => '';

function stripped(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = stripped(css);
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    return out;
  }
  return undefined;
}

function media(css: string, query: string): string {
  const src = stripped(css);
  const at = src.indexOf(`@media (${query})`);
  if (at < 0) return '';
  const open = src.indexOf('{', at);
  let depth = 1;
  let i = open + 1;
  while (i < src.length && depth > 0) {
    if (src[i] === '{') depth += 1;
    if (src[i] === '}') depth -= 1;
    i += 1;
  }
  return src.slice(open + 1, i - 1);
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  readHere = (name) => fs.readFileSync(here + name, 'utf8');
});

describe('shared card at phone width', () => {
  it('CP-1 the base card keeps 20px', () => {
    expect(decls(readHere('card.css'), '.xc-theme .card')?.get('padding')).toBe('20px');
  });

  it('CP-2 at 800px or less the base card pads 16px', () => {
    const m = media(readHere('card.css'), 'max-width: 800px');
    expect(m, '@media (max-width: 800px) in card.css').not.toBe('');
    expect(decls(m, '.xc-theme .card')?.get('padding')).toBe('16px');
  });

  it('CP-3 the phone rule changes padding only', () => {
    const d = decls(media(readHere('card.css'), 'max-width: 800px'), '.xc-theme .card');
    expect([...(d?.keys() ?? [])]).toEqual(['padding']);
  });
});
