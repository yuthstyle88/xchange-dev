// OL-7 row 1 re-draft on feat/one-look-7 (VERIFY.md on main 4a577dac: still NOT FIXED). Same test as test/ol6-b-login-alert
// (bd01bea3, ticket #6697, never done). Written before the fix by claude/studio-claude-17. Do not edit, delete or weaken.
// OL-6 B login-alert (audit #564 issuecomment-6702970101, shot 001): the staff sign-in server error `#message` was
// bare dark-red text with an empty 1em-margin line always present. It becomes an alert box, hidden while empty.
// auth.css is imported only by AuthLayout (staff sign-in); customer sign-in does not load it (ruling A1).
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

describe('staff sign-in alert', () => {
  it('LA-1 #message is an alert box: danger-soft fill, danger line and text, radius, padding', () => {
    const d = decls(readHere('auth.css'), '.xc-auth #message');
    expect(d?.get('background')).toBe('var(--xc-danger-soft)');
    expect(d?.get('border')).toBe('1px solid var(--xc-danger)');
    expect(d?.get('color')).toBe('var(--xc-danger)');
    expect(d?.get('border-radius')).toBe('var(--xc-radius-md)');
    expect(d?.get('padding')).toBe('10px 14px');
    expect(d?.get('margin')).toBe('16px 0 0');
    expect(d?.get('font-size')).toBe('14px');
  });

  it('LA-2 an empty #message is not shown', () => {
    expect(decls(readHere('auth.css'), '.xc-auth #message:empty')?.get('display')).toBe('none');
  });
});
