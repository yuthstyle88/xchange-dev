// OL-6 B admin-css-1 (audit #564 issuecomment-6702970101, shots 018 and 030/034/037/038): two admin.css leftovers.
// The period toolbar's date rule lost to the A3 field rule (`.xc-content.xc-content input:not(…)`, width 100%), so
// both dates stacked full width; and the overview status strip was a grey box, not a white card.
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

const css = () => readHere('admin.css');

describe('admin.css period toolbar and status strip', () => {
  it('AC-1 the date fields beat the A3 field rule: 168px wide, 44px tall, side by side', () => {
    const phone = media(css(), 'max-width: 600px');
    const wide = stripped(css()).replace(phone, '');
    const d = decls(wide, '.xc-admin .xc-content.xc-content .period-toolbar input[type="date"]');
    expect(d, 'date rule carries the doubled .xc-content prefix').toBeDefined();
    expect(d?.get('width')).toBe('168px');
    expect(d?.get('min-height')).toBe('44px');
    expect(d?.get('font-size')).toBe('14px');
    expect(decls(wide, '.xc-admin .xc-content .period-toolbar input[type="date"]'), 'the losing single-prefix rule is gone').toBeUndefined();
    expect(wide.indexOf('.xc-content.xc-content .period-toolbar input[type="date"]')).toBeGreaterThan(
      wide.indexOf('.xc-content.xc-content input:not([type="checkbox"]):not([type="radio"])'),
    );
  });

  it('AC-5 at 600px or less the two dates share one row', () => {
    const phone = media(css(), 'max-width: 600px');
    const d = decls(phone, '.xc-admin .xc-content.xc-content .period-toolbar input[type="date"]');
    expect(d?.get('flex')).toBe('1 1 130px');
    expect(d?.get('width')).toBe('auto');
    expect(d?.get('min-width')).toBe('0');
    expect(decls(phone, '.xc-admin .xc-content .period-toolbar input[type="date"]'), 'no losing phone rule').toBeUndefined();
  });

  it('AC-2 วันนี้ / ทั้งหมด get room: 8px 14px, 13px', () => {
    const d = decls(css(), '.xc-admin .xc-content .period-toolbar button');
    expect(d?.get('padding')).toBe('8px 14px');
    expect(d?.get('font-size')).toBe('13px');
  });

  it('AC-3 the toolbar is a card: theme radius and shadow', () => {
    const d = decls(css(), '.xc-admin .xc-content .period-toolbar');
    expect(d?.get('background')).toBe('var(--xc-surface)');
    expect(d?.get('border-radius')).toBe('var(--xc-radius-xl)');
    expect(d?.get('box-shadow')).toBe('var(--xc-shadow-sm)');
  });

  it('AC-4 the status strip is a white card', () => {
    const d = decls(css(), '.xc-admin .xc-content .status-strip');
    expect(d?.get('background')).toBe('var(--xc-surface)');
    expect(d?.get('border')).toBe('1px solid var(--xc-line)');
    expect(d?.get('border-radius')).toBe('var(--xc-radius-xl)');
    expect(d?.get('box-shadow')).toBe('var(--xc-shadow-sm)');
  });
});
