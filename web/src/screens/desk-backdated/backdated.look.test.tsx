// OL-6 L2 (design docs/design/xchange-one-look-layout.md §1 D3, §3 L2; mock-up docs/design/one-look-mock/index.html:170-196):
// /desk/backdated is one stacked column with no card. The form becomes a card with the amount fields in a row of three,
// and the figures the screen already computes sit in a summary card beside it. Layout only: no new field, no new value.
// Written before the fix by claude/bbe9ce8b. Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { BackdatedScreen } from './BackdatedScreen';

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
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(css)) !== null) {
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

/** The body of `@media (max-width: <px>px)`, comments removed. */
function media(css: string, px: number): string {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const at = stripped.search(new RegExp(`@media\\s*\\(max-width:\\s*${px}px\\)\\s*\\{`));
  if (at < 0) return '';
  let i = stripped.indexOf('{', at) + 1;
  const start = i;
  for (let depth = 1; depth > 0 && i < stripped.length; i += 1) {
    if (stripped[i] === '{') depth += 1;
    if (stripped[i] === '}') depth -= 1;
  }
  return stripped.slice(start, i - 1);
}

const plain = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '');

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  const api = new ApiClient(
    async () =>
      new Response(JSON.stringify({ version: 1, changed: true, data: { currencies: [], rates: [] } }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
  );
  dispose = render(() => <BackdatedScreen api={api} me={() => undefined} />, host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
});

const named = (name: string) => host.querySelector(`[name="${name}"]`);
const fieldOf = (name: string) => named(name)?.closest('.desk-field');

describe('L2 /desk/backdated cards', () => {
  it('L2-1 the screen loads the shared card and its own stylesheet after desk.css', async () => {
    const src = await readHere('BackdatedScreen.tsx');
    expect(src).toMatch(/import '\.\.\/\.\.\/shared\/ui\/card\.css';/);
    expect(src).toMatch(/import '\.\/backdated\.css';/);
    expect(src.indexOf("'./backdated.css'")).toBeGreaterThan(src.indexOf("'../desk/desk.css'"));
  });

  it('L2-2 the form is a card inside a two-column grid, beside the summary card', () => {
    expect(host.querySelector('.desk-page > .backdated-grid > form.desk-form.backdated-form.card')).not.toBeNull();
    expect(host.querySelector('.desk-page > .backdated-grid > aside.backdated-summary.card')).not.toBeNull();
  });

  it('L2-3 the computed figures moved to the summary card, and only they did', () => {
    const line = host.querySelector('[data-testid="computed-thb"]');
    expect(line?.closest('aside.backdated-summary')).not.toBeNull();
    expect(host.querySelector('aside.backdated-summary input, aside.backdated-summary select')).toBeNull();
  });

  it('L2-4 currency, quantity and rate share a row of three, in that order', () => {
    const row = fieldOf('currency')?.parentElement;
    expect(row?.classList.contains('backdated-grid3')).toBe(true);
    expect(Array.from(row!.children).map((c) => c.querySelector('[name]')?.getAttribute('name'))).toEqual([
      'currency',
      'quantity',
      'rate',
    ]);
  });

  it.each([
    ['feeThb', 'expectedSettleThb'],
    ['reference', 'account'],
    ['slip', 'passport'],
  ])('L2-5 %s and %s share a row of two', (a, b) => {
    const row = fieldOf(a)?.parentElement;
    expect(row?.classList.contains('backdated-grid2')).toBe(true);
    expect(fieldOf(b)?.parentElement).toBe(row);
  });

  it('L2-6 every field is still inside the form, so submitting still reads them', () => {
    const form = host.querySelector('form');
    for (const name of ['customerSearch', 'customerId', 'side', 'currency', 'quantity', 'rate', 'feeThb', 'expectedSettleThb', 'paidAt', 'method', 'reference', 'account', 'bankFeeThb', 'slip', 'passport', 'notes']) {
      expect(named(name)?.closest('form'), name).toBe(form);
    }
  });

  it('L2-7 the outer grid is two equal columns, the summary keeps to its content', async () => {
    const css = plain(await readHere('backdated.css'));
    const grid = decls(css, '.backdated-grid');
    expect(grid?.get('display')).toBe('grid');
    expect(grid?.get('grid-template-columns')).toBe('minmax(0, 1fr) minmax(0, 1fr)');
    expect(grid?.get('gap')).toBe('16px');
    expect(decls(css, '.backdated-summary')?.get('align-self')).toBe('start');
  });

  it('L2-8 the form card drops desk.css\'s 560px limit: the card column gives the width', async () => {
    const css = plain(await readHere('backdated.css'));
    expect(decls(css, '.desk-form.backdated-form')?.get('max-width')).toBe('none');
  });

  it('L2-9 the field rows use the mock-up\'s columns and let their fields shrink', async () => {
    const css = plain(await readHere('backdated.css'));
    expect(decls(css, '.backdated-grid2')?.get('grid-template-columns')).toBe('repeat(2, minmax(0, 1fr))');
    expect(decls(css, '.backdated-grid3')?.get('grid-template-columns')).toBe('repeat(3, minmax(0, 1fr))');
    for (const sel of ['.backdated-grid2', '.backdated-grid3']) {
      expect(decls(css, sel)?.get('gap'), sel).toBe('12px');
      expect(decls(css, sel)?.get('align-items'), sel).toBe('end');
    }
  });

  it('L2-10 at 1100px or less the cards stack with the summary first, so the figures stay in view', async () => {
    const block = media(await readHere('backdated.css'), 1100);
    expect(decls(block, '.backdated-grid')?.get('grid-template-columns')).toBe('minmax(0, 1fr)');
    expect(decls(block, '.backdated-summary')?.get('order')).toBe('-1');
  });

  it('L2-11 at 560px or less the field rows stack', async () => {
    const block = media(await readHere('backdated.css'), 560);
    for (const sel of ['.backdated-grid2', '.backdated-grid3']) {
      expect(decls(block, sel)?.get('grid-template-columns'), sel).toBe('minmax(0, 1fr)');
    }
  });

  it('L2-12 the search field fills its row beside its button', async () => {
    const row = named('customerSearch')?.parentElement;
    expect(row?.classList.contains('backdated-search')).toBe(true);
    const css = plain(await readHere('backdated.css'));
    expect(decls(css, '.backdated-search')?.get('flex-wrap')).toBe('nowrap');
    expect(decls(css, '.backdated-search')?.get('margin-top')).toBe('0');
    const input = decls(css, '.backdated-search input');
    expect(input?.get('flex')).toBe('1 1 0');
    expect(input?.get('min-width')).toBe('0');
  });
});
