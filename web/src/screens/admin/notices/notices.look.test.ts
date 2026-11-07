// OL-6 B admin-notices (audit #564 issuecomment-6702970101, shot 050): A3 fixed the Thai eyebrow; left are the disabled
// "อ่านทั้งหมดแล้ว" at half opacity on top of the theme's solid disabled colours, and the column centred 30px right of
// the heading.
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

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  readHere = (name) => fs.readFileSync(here + name, 'utf8');
});

describe('notices look', () => {
  it('NO-1 disabled buttons keep the solid disabled colours: no opacity', () => {
    const css = readHere('notices.css');
    for (const sel of ['.admin-notices-mark-all-btn:disabled', '.admin-notices-mark-btn:disabled']) {
      const d = decls(css, sel);
      expect(d, sel).toBeDefined();
      expect(d?.has('opacity'), `${sel} has no opacity`).toBe(false);
    }
    expect(stripped(css)).not.toMatch(/opacity\s*:\s*0?\.5/);
  });

  it('NO-2 the column starts under the heading (no auto margin, no extra padding)', () => {
    const d = decls(readHere('notices.css'), '.admin-notices');
    expect(d?.has('margin')).toBe(false);
    expect(d?.has('padding')).toBe(false);
    expect(d?.get('max-width')).toBe('56rem');
  });
});
