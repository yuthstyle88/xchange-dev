// OL-6 wave B admin-pricing (audit #564 issuecomment-6702970101, shot 024), re-measured on feat/one-look-6 after A5:
// - "สกุลแนะนำ" adds a currency from a select whose first choice was an empty <option />: a blank box with no hint of
//   what it does. Its first choice now says "+ เพิ่มสกุลแนะนำ" / "+ Add a featured currency", which also names the select.
// - The bulk "อนุมัติที่เลือก" button, disabled while nothing is approvable, kept its brand fill at opacity 0.5 (a pink
//   block): `.bulk-approve .bulk-approve-btn` outranks admin.css's solid disabled rule (A3). It now shows that look.
// OL-7 (verification of 2026-11-07, rows 50 and 51; re-measured on feat/one-look-7 06b1e00b with xc-v3 data):
// - The market-source chip rail scrolls sideways on narrow screens but hides its scrollbar (`scrollbar-width: none` plus
//   a `::-webkit-scrollbar { display: none }` rule), so nothing says more chips are there. It now shows a thin bar.
// - pricing-row.css sets the secondary text (sources, BoT, status notes) at 11px, under the admin's 12px floor.
// Written before the fix by claude/bbe9ce8b for the OL sub-controller claude/1cc51615 (OL-6, carried to OL-7). Do not edit, delete or weaken.
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { makePricingDisplayDto } from '../../../shared/testing/fixtures/pricing_board';
import { boardFetch } from './__fixtures__/pricing-board.fixture';
import { DisplaySettings } from './DisplaySettings';
import { en, th } from './i18n';

let host: HTMLDivElement | undefined;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
  setLang('th');
});

function mount() {
  const server = boardFetch(undefined, () => ({ status: 404, body: {} }));
  const api = new ApiClient(server.fn as unknown as typeof fetch);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <DisplaySettings
        api={api}
        branch="B01"
        value={makePricingDisplayDto({ tiers: [1], featured: ['USD'], highlight: false, allMarket: false, from: 'MAIN' })}
        currencies={['USD', 'EUR', 'JPY']}
        mayEdit={true}
        onSaved={() => undefined}
      />
    ),
    host,
  );
}

const addSelect = () => host!.querySelector<HTMLSelectElement>('select[name="featured-add"]')!;

describe('admin-pricing: the featured-currency select says what it does', () => {
  it('PA-1 the first choice keeps value "" and reads + เพิ่มสกุลแนะนำ (th)', () => {
    mount();
    const first = addSelect().options[0]!;
    expect(first.value).toBe('');
    expect(first.textContent?.trim()).toBe('+ เพิ่มสกุลแนะนำ');
  });

  it('PA-2 the select is named by the same text (aria-label)', () => {
    mount();
    expect(addSelect().getAttribute('aria-label')).toBe('+ เพิ่มสกุลแนะนำ');
  });

  it('PA-3 the key is displayFeaturedAdd in both tables (th / en)', () => {
    expect((th as Record<string, string>).displayFeaturedAdd).toBe('+ เพิ่มสกุลแนะนำ');
    expect((en as Record<string, string>).displayFeaturedAdd).toBe('+ Add a featured currency');
  });

  it('PA-4 in English the first choice reads + Add a featured currency', () => {
    setLang('en');
    mount();
    expect(addSelect().options[0]!.textContent?.trim()).toBe('+ Add a featured currency');
  });
});

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

/** Every rule whose selector list contains `selector` exactly, merged in source order. */
function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  let out: Map<string, string> | undefined;
  while ((m = re.exec(stripped)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    out ??= new Map();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
  }
  return out;
}

describe('admin-pricing: a disabled bulk button is solid grey', () => {
  let css = '';
  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    // resolve next to this file: CI runs vitest with --root web from the repo root
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    css = fs.readFileSync(here + 'bulk-approve.css', 'utf8');
  });

  for (const which of ['bulk-approve-btn', 'bulk-approve-cancel']) {
    it(`PA-5 .${which}:disabled under .bulk-approve at the scoped weight: opacity 1, gray-100 / line / gray-600, no shadow`, () => {
      const d = decls(css, `.xc-admin .xc-content.xc-content .bulk-approve .${which}:disabled`);
      expect(d, `rule \`.xc-admin .xc-content.xc-content .bulk-approve .${which}:disabled\``).toBeDefined();
      expect(d!.get('opacity')).toBe('1');
      expect(d!.get('background')).toBe('var(--xc-gray-100)');
      expect(d!.get('border-color')).toBe('var(--xc-line)');
      expect(d!.get('color')).toBe('var(--xc-gray-600)');
      expect(d!.get('box-shadow')).toBe('none');
    });
  }
});

describe('admin-pricing OL-7: the chip rail shows that it scrolls', () => {
  let css = '';
  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    css = fs.readFileSync(here + 'market-sources.css', 'utf8');
  });

  it('PA-6 .xc-market-sources keeps a thin scrollbar, and no rule hides its webkit scrollbar', () => {
    const d = decls(css, '.xc-admin .xc-content.xc-content .xc-market-sources');
    expect(d, 'rule `.xc-admin .xc-content.xc-content .xc-market-sources`').toBeDefined();
    expect(d!.get('overflow-x')).toBe('auto');
    expect(d!.get('scrollbar-width')).toBe('thin');
    expect(css.replace(/\/\*[\s\S]*?\*\//g, '')).not.toMatch(/xc-market-sources::-webkit-scrollbar/);
  });
});

describe('admin-pricing OL-7: no pricing-row text under 12px', () => {
  let css = '';
  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    css = fs.readFileSync(here + 'pricing-row.css', 'utf8');
  });

  it('PA-7 every font-size in pricing-row.css is a px value of 12 or more', () => {
    const sizes = [...css.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/font-size\s*:\s*([^;}]+)/g)].map((m) => m[1]!.trim());
    expect(sizes.length).toBeGreaterThan(0);
    for (const v of sizes) {
      expect(v, `font-size: ${v}`).toMatch(/^\d+(\.\d+)?px$/);
      expect(parseFloat(v), `font-size: ${v}`).toBeGreaterThanOrEqual(12);
    }
  });
});
