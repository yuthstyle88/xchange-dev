// OL-6 B admin-alert-settings (audit #564 issuecomment-6702970101, shot 048): the card was inset 24px from the heading,
// "เพิ่มเกณฑ์" was grey-filled unlike every other secondary, and "no floors" was a bare line of text.
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

describe('alert-settings look', () => {
  it('AS-1 the container adds no padding, so the card lines up with the heading', () => {
    const d = decls(readHere('alert-settings.css'), '.alert-settings-container');
    expect(d?.has('padding')).toBe(false);
    expect(d?.get('max-width')).toBe('640px');
  });

  it('AS-2 the card is the shared card with 20px padding', () => {
    const tsx = readHere('AlertSettings.tsx');
    expect(tsx).toContain("import '../../../shared/ui/card.css';");
    expect(tsx).toContain('<div class="alert-settings-card card">');
    const css = readHere('alert-settings.css');
    const d = decls(css, '.xc-admin .xc-content .alert-settings-card');
    expect(d?.get('padding')).toBe('20px');
    expect(d?.get('margin-bottom')).toBe('0');
    expect(decls(css, '.alert-settings-card'), 'the old copy of the card values is gone').toBeUndefined();
  });

  it('AS-3 "เพิ่มเกณฑ์" is a secondary: white with brand text', () => {
    const d = decls(readHere('alert-settings.css'), '.add-floor-button');
    expect(d?.get('background')).toBe('var(--xc-surface)');
    expect(d?.get('color')).toBe('var(--xc-brand)');
  });

  it('AS-4 no floors is an empty-state box', () => {
    const d = decls(readHere('alert-settings.css'), '.no-floors');
    expect(d?.get('background')).toBe('var(--xc-gray-50)');
    expect(d?.get('border')).toBe('1px dashed var(--xc-line)');
    expect(d?.get('border-radius')).toBe('var(--xc-radius-md)');
    expect(d?.get('text-align')).toBe('center');
  });

  it('AS-5 save is the primary button', () => {
    expect(readHere('AlertSettings.tsx')).toMatch(/<button\s+type="button"\s+class="primary"\s+disabled=\{!firstLoaded\(\) \|\| saving\(\)\}/);
  });
});
