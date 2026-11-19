// OL-6 wave B staff-cards (audit #564 issuecomment-6702970101, shots 010-012): the /staff toolbar and the card frames.
// Card radius 8px not 12; the transfers card has no inner padding; rates numbers are monospace next to sans; the holds
// form is a grey box inside a white card; toolbar buttons are 38px. Written before the fix by claude/bbe9ce8b. Do not
// edit, delete or weaken.
import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

async function readHere(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8');
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

function media800(css: string, selector: string): string {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const block = /@media\s*\(max-width:\s*800px\)\s*\{([\s\S]*?\})\s*\}/.exec(stripped)?.[1] ?? '';
  return new RegExp(selector.replace('.', '\\.') + '\\s*\\{([^}]*)\\}').exec(block)?.[1] ?? '';
}

describe('staff-cards look', () => {
  it('SC-1 the card frame has the shared card radius', async () => {
    expect(decls(await readHere('staff.css'), '.xc-staff-card-wrapper')?.get('border-radius')).toBe('var(--xc-radius-xl)');
  });

  it('SC-2 the transfers card has inner padding: 20px, 16px at 800px or less', async () => {
    const css = await readHere('transfers/transfers.css');
    expect(decls(css, '.xc-transfers-card')?.get('padding')).toBe('20px');
    expect(media800(css, '.xc-transfers-card')).toMatch(/padding\s*:\s*16px/);
  });

  it('SC-3 the transfers table has room, scrolls when narrow, and its buttons do not wrap', async () => {
    const css = await readHere('transfers/transfers.css');
    expect(decls(css, '.xc-transfers-table td')?.get('padding')).toBe('12px 8px');
    expect(decls(css, '.xc-transfers-list')?.get('overflow-x')).toBe('auto');
    expect(decls(css, '.xc-transfers-table button')?.get('white-space')).toBe('nowrap');
  });

  it('SC-4 rates numbers are the sans font with tabular figures, not monospace', async () => {
    const d = decls(await readHere('rates/rates.css'), '.xc-rates-td-buy');
    expect(d?.has('font-family')).toBe(false);
    expect(d?.get('font-variant-numeric')).toBe('tabular-nums');
  });

  it('SC-5 the holds form is not a box inside the card', async () => {
    const d = decls(await readHere('holds/holds.css'), '.xc-holds-form');
    for (const prop of ['background', 'border', 'border-radius', 'padding']) {
      expect(d?.has(prop), `.xc-holds-form must not set ${prop}`).toBe(false);
    }
  });

  it('SC-6 the holds inputs leave height, radius and border to the theme', async () => {
    const d = decls(await readHere('holds/holds.css'), '.xc-holds-input');
    expect(d?.get('width')).toBe('100%');
    for (const prop of ['padding', 'font-size', 'border', 'border-radius', 'background']) {
      expect(d?.has(prop), `.xc-holds-input must not set ${prop}`).toBe(false);
    }
  });

  it('SC-7 the holds buttons have the 44px shape: search secondary, save primary, disabled solid', async () => {
    const css = await readHere('holds/holds.css');
    for (const sel of ['.xc-holds-btn', '.xc-holds-save-btn']) {
      const d = decls(css, sel);
      expect(d?.get('min-height'), sel).toBe('44px');
      expect(d?.get('padding'), sel).toBe('10px 18px');
      expect(d?.get('border-radius'), sel).toBe('var(--xc-radius-md)');
      expect(d?.get('font-size'), sel).toBe('15px');
      expect(d?.get('font-weight'), sel).toBe('600');
    }
    expect(decls(css, '.xc-holds-btn')?.get('color')).toBe('var(--xc-brand-text)');
    expect(decls(css, '.xc-holds-save-btn')?.get('border')).toBe('1px solid var(--xc-brand)');
    expect(decls(css, '.xc-holds-save-btn:disabled')?.has('opacity') ?? false).toBe(false);
  });

  it('SC-8 the toolbar controls are 44px and the header aligns to the centre', async () => {
    const css = await readHere('staff.css');
    expect(decls(css, '.xc-staff-header')?.get('align-items')).toBe('center');
    for (const sel of ['.xc-staff-refresh-btn', '.xc-staff-report-link']) {
      const d = decls(css, sel);
      expect(d?.get('min-height'), sel).toBe('44px');
      expect(d?.get('padding'), sel).toBe('10px 18px');
      expect(d?.get('border-radius'), sel).toBe('var(--xc-radius-md)');
      expect(d?.get('font-size'), sel).toBe('15px');
      expect(d?.get('font-weight'), sel).toBe('600');
      expect(d?.get('box-sizing'), sel).toBe('border-box');
    }
    expect(decls(css, '.xc-staff-refresh-btn')?.get('color')).toBe('var(--xc-brand-text)');
  });
});
