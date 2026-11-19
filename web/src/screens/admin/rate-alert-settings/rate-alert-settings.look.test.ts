// OL-6 B admin-rate-alert-settings (audit #564 issuecomment-6702970101, shot 049): A3 fixed the radios; left are the
// amber notice touching the card, "ใช้ค่าแนะนำ" as bare black text, and the column centred 94px right of the heading.
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

describe('rate-alert-settings look', () => {
  it('RA-1 the column starts under the heading (no auto margin, no extra padding)', () => {
    const d = decls(readHere('rate-alert-settings.css'), '.rate-alert-settings');
    expect(d?.has('margin')).toBe(false);
    expect(d?.has('padding')).toBe(false);
    expect(d?.get('max-width')).toBe('48rem');
  });

  it('RA-2 banner, success note and card are spaced 16px apart', () => {
    expect(readHere('RateAlertSettings.tsx')).toContain(
      `<div class="rate-alert-body" data-state={isEmpty() ? 'empty' : 'loaded'}>`,
    );
    const d = decls(readHere('rate-alert-settings.css'), '.rate-alert-body');
    expect(d?.get('display')).toBe('flex');
    expect(d?.get('flex-direction')).toBe('column');
    expect(d?.get('gap')).toBe('16px');
  });

  it('RA-3 the card is the shared card with 20px padding', () => {
    const tsx = readHere('RateAlertSettings.tsx');
    expect(tsx).toContain("import '../../../shared/ui/card.css';");
    expect(tsx).toContain('<div class="rate-alert-card card">');
    const css = readHere('rate-alert-settings.css');
    expect(decls(css, '.xc-admin .xc-content .rate-alert-card')?.get('padding')).toBe('20px');
    expect(decls(css, '.rate-alert-card'), 'the old copy of the card values is gone').toBeUndefined();
  });

  it('RA-4 "ใช้ค่าแนะนำ" is a secondary: white with brand text', () => {
    const d = decls(readHere('rate-alert-settings.css'), '.rate-alert-suggest-btn');
    expect(d?.get('background-color')).toBe('var(--xc-surface)');
    expect(d?.get('color')).toBe('var(--xc-brand-text)');
  });
});
