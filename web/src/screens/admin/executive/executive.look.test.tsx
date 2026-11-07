// OL-6 wave B admin (audit #564 issuecomment-6702970101, row 034): the executive overview's look. The branch table sits
// in a card and scrolls inside it, the drill-down button is not underlined, the branch filter is a field with chip
// checkboxes instead of a browser fieldset, and the currency field says what an empty value means. Written before the
// fix by claude/94389a6b. Do not edit, delete or weaken; add your own elsewhere.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeExecutiveOverviewData } from '../../../shared/testing/fixtures';
import { Executive } from './Executive';
import { en, th } from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

function decls(selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (m[1]!.trim() !== selector) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'executive.css', 'utf8');
});

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(async () => {
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            version: 1,
            versions: { B1: 1 },
            changed: true,
            data: makeExecutiveOverviewData({ period: { from: '2026-11-07', to: '2026-11-07' } }),
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
    ),
  );
  host = document.createElement('div');
  document.body.appendChild(host);
  const [branch] = createSignal('B1');
  const [period] = createSignal({ from: '2026-11-07', to: '2026-11-07' });
  const [shown] = createSignal(true);
  dispose = render(() => <Executive branch={branch} period={period} shown={shown} />, host);
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

describe('executive look', () => {
  it('EX-1 the branch table is in a card and scrolls inside it', () => {
    const table = host.querySelector('table.executive-branches');
    expect(table, 'the branch table rendered').not.toBeNull();
    expect(table!.closest('.card .table-scroll')).not.toBeNull();
  });

  it('EX-2 the table cell rule the admin table rule always beat is gone', () => {
    expect(decls('.executive-branches th,\n.executive-branches td')).toBeUndefined();
  });

  it('EX-3 the drill-down button is not underlined and copies no button values the admin rule overrides', () => {
    const d = decls('.executive-drill-link');
    expect(d?.get('text-decoration')).toBe('none');
    for (const p of ['padding', 'border', 'background']) expect(d?.has(p), p).toBe(false);
  });

  it('EX-4 the branch filter is a plain field: no fieldset frame, the legend reads like a label', () => {
    const f = decls('.xc-admin .xc-content .executive-branch-filter');
    expect(f?.get('border')).toBe('0');
    expect(f?.get('padding')).toBe('0');
    expect(f?.get('margin')).toBe('0');
    expect(f?.get('min-width')).toBe('0');
    const l = decls('.xc-admin .xc-content .executive-branch-filter legend');
    expect(l?.get('padding')).toBe('0');
    expect(l?.get('margin-bottom')).toBe('6px');
    expect(l?.get('font-size')).toBe('14px');
    expect(l?.get('font-weight')).toBe('500');
  });

  it('EX-5 each branch is a 44px chip, marked when checked', () => {
    const c = decls('.xc-admin .xc-content .executive-check');
    expect(c?.get('min-height')).toBe('44px');
    expect(c?.get('padding')).toBe('0 12px');
    expect(c?.get('border')).toBe('1px solid var(--xc-line)');
    expect(c?.get('border-radius')).toBe('7px');
    expect(c?.get('background')).toBe('var(--xc-surface)');
    const on = decls('.xc-admin .xc-content .executive-check:has(input:checked)');
    expect(on?.get('border-color')).toBe('var(--xc-brand)');
    expect(on?.get('background')).toBe('var(--xc-brand-soft)');
  });

  it('EX-6 the currency field says that empty means every currency, in both languages', () => {
    expect(th.currencyPlaceholder).toBe('ทุกสกุล เช่น USD');
    expect(en.currencyPlaceholder).toBe('All, e.g. USD');
    expect(host.querySelector<HTMLInputElement>('input[name="currency"]')?.placeholder).toBe(th.currencyPlaceholder);
  });

  it('EX-7 the field rule the admin field rule always beat is gone', () => {
    expect(decls('.executive-field input,\n.executive-field select')).toBeUndefined();
  });
});
