// OL-6 wave B admin-pricing-formula (audit #564 issuecomment-6702970101, shot 025): the screen had no card at all.
// - Three white cards (`section.card`): the formula (header, editor, preview), the branch table, the reason + save.
// - The branch table sits in `.table-scroll`: a long branch id no longer pushes the phone page to 572 px.
// - The main save is the page's primary action (`button.primary`), not a white secondary.
// - The branch column printed the raw id. The view carries no branch name (`pricing_formula` rows are
//   `{ branch, source, own }`), so no label is invented: an id longer than 12 characters (a UUID) shows its first 8 and
//   "…", with the whole id in `title`; short ids (MAIN, B2) print as they are.
// Written before the fix by claude/bbe9ce8b for the OL sub-controller claude/1cc51615 (OL-6, carried to OL-7). Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makePricingFormulaData } from '../../../shared/testing/fixtures';
import type { AdminMenuProps } from '../menus';
import { resetFormulaStubs } from '../parts/formula-stubs.testing';
import * as screen from './PricingFormula';

vi.mock('../parts/FormulaEditor', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/FormulaPreviewTable', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/FormulaRefusal', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/formula-preview', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/spread-failure', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/toast', () => ({ toast: vi.fn(), Toast: () => null }));

const VIEW = '/api/v3/views/pricing_formula';
const UUID = 'B9082fb61-5ff1-41f9-b4e4-c4507b926544';
const LONG_NAME = 'สาขาเซ็นทรัลเวิลด์ ชั้น 3';

function data() {
  return makePricingFormulaData({
    organisation: null,
    default: {
      buyFormula: 'ref_buy * (1 + (buy_pct + tier_buy_pct) / 100)',
      sellFormula: 'ref_sell * (1 + (sell_pct + tier_sell_pct) / 100)',
    },
    inputs: ['ref_buy', 'ref_sell'].map((name) => ({ name })),
    branches: [
      { branch: UUID, name: LONG_NAME, source: 'DEFAULT', own: null },
      { branch: 'MAIN', name: 'สำนักงานใหญ่', source: 'DEFAULT', own: null },
      { branch: 'B2', name: 'สาขาสยาม', source: 'DEFAULT', own: null },
    ],
  });
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  resetFormulaStubs();
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url.startsWith(VIEW)) return json({ version: 1, changed: true, data: data() });
      return json({ error: { code: 'notFound', message: url } }, 404);
    }),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

async function mount(): Promise<void> {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  const props: AdminMenuProps = { branch, period, shown };
  dispose = render(() => <screen.PricingFormula {...props} />, host);
  await vi.waitFor(() => expect(host.querySelector('textarea[data-field="buyFormula"]')).not.toBeNull());
}

const cards = () => [...host.querySelectorAll<HTMLElement>('.pricing-formula-screen > section.card')];

describe('admin-pricing-formula: three cards', () => {
  it('PF-1 the screen holds exactly three section.card, in order: formula, branches, reason + save', async () => {
    await mount();
    const c = cards();
    expect(c, 'three cards directly under .pricing-formula-screen').toHaveLength(3);
    expect(c[0]!.classList.contains('pricing-formula-editor')).toBe(true);
    expect(c[0]!.querySelector('[data-testid="formula-current"]'), 'header in the formula card').not.toBeNull();
    expect(c[0]!.querySelector('textarea[data-field="buyFormula"]'), 'editor in the formula card').not.toBeNull();
    expect(c[1]!.querySelector('table.pricing-formula-branches'), 'branch table in the second card').not.toBeNull();
    expect(c[2]!.classList.contains('pricing-formula-save')).toBe(true);
    expect(c[2]!.querySelector('textarea[data-field="reason"]'), 'reason in the last card').not.toBeNull();
    expect(c[2]!.querySelector('button[data-action="formula-save"]'), 'save in the last card').not.toBeNull();
  });

  it('PF-2 the branch table scrolls inside its own .table-scroll', async () => {
    await mount();
    const table = host.querySelector('table.pricing-formula-branches')!;
    expect(table.parentElement?.classList.contains('table-scroll')).toBe(true);
  });

  it('PF-3 the save button is the primary action', async () => {
    await mount();
    expect(host.querySelector('button[data-action="formula-save"]')!.classList.contains('primary')).toBe(true);
  });
});

describe('admin-pricing-formula: branches by name (OL-8 row 54)', () => {
  it('PF-4 branchLabel is gone: the screen no longer shortens ids', () => {
    expect((screen as unknown as { branchLabel?: unknown }).branchLabel, 'PricingFormula.tsx exports no branchLabel').toBeUndefined();
  });

  it('PF-5 each row prints the branch name from the view, whole, with the branch id in title and data-branch', async () => {
    await mount();
    const cell = (id: string) => host.querySelector<HTMLElement>(`tr[data-branch="${id}"] td`)!;
    expect(cell(UUID).textContent?.trim()).toBe(LONG_NAME);
    expect(cell(UUID).getAttribute('title')).toBe(UUID);
    expect(cell('MAIN').textContent?.trim()).toBe('สำนักงานใหญ่');
    expect(cell('MAIN').getAttribute('title')).toBe('MAIN');
    expect(cell('B2').textContent?.trim()).toBe('สาขาสยาม');
    expect(cell('B2').getAttribute('title')).toBe('B2');
  });
});

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
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

describe('admin-pricing-formula: pricing-formula.css', () => {
  let css = '';
  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    // resolve next to this file: CI runs vitest with --root web from the repo root
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    css = fs.readFileSync(here + 'pricing-formula.css', 'utf8');
  });

  it("PF-6 the cards drop card.css's 22px bottom margin; the screen's gap spaces them", () => {
    const d = decls(css, '.xc-admin .xc-content .pricing-formula-screen > .card');
    expect(d, 'rule `.xc-admin .xc-content .pricing-formula-screen > .card`').toBeDefined();
    expect(d!.get('margin-bottom')).toBe('0');
  });

  it('PF-7 the formula and save cards stack their parts with a 1rem gap', () => {
    for (const sel of ['.xc-admin .xc-content .pricing-formula-editor', '.xc-admin .xc-content .pricing-formula-save']) {
      const d = decls(css, sel);
      expect(d, `rule \`${sel}\``).toBeDefined();
      expect(d!.get('display')).toBe('flex');
      expect(d!.get('flex-direction')).toBe('column');
      expect(d!.get('gap')).toBe('1rem');
    }
  });
});
