// Pricing formula FE T5 (docs/design/blueprint/pricing-formula-web.md §2, §3 R1–R2, #5577)
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { SpreadFailureLine } from './spread-failure';
import { FormulaRefusal } from './FormulaRefusal';
import * as formulaTable from './formula-i18n';

vi.mock('./spread-failure', () => import('./formula-stubs.testing'));

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(fn: () => JSX.Element): HTMLElement {
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(fn, root);
  return root;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  setLang('th');
});

beforeEach(() => {
  setLang('th');
});

function makeLine(over: Partial<SpreadFailureLine> = {}): SpreadFailureLine {
  return {
    branch: 'MAIN',
    currency: 'USD',
    tier: null,
    follow: false,
    kind: 'buyAboveSell',
    buy: '33.8837374',
    sell: '33.7293303',
    side: null,
    guardMin: null,
    guardMax: null,
    pct: null,
    code: null,
    raw: 'MAIN USD buy 33.8837374 > sell 33.7293303',
    ...over,
  };
}

describe('FormulaRefusal (T5, #5577)', () => {
  it('R1: lines given renders the refusal block with one row per line, in order', () => {
    const line1 = makeLine({ branch: 'MAIN', currency: 'USD', raw: 'failure line 1' });
    const line2 = makeLine({ branch: 'BRANCH-2', currency: 'JPY', tier: 2, raw: 'failure line 2' });

    const el = mount(() => <FormulaRefusal lines={() => [line1, line2]} />);

    const alert = el.querySelector('[role="alert"].formula-refusal');
    expect(alert).not.toBeNull();

    const heading = alert!.querySelector('h3');
    expect(heading).not.toBeNull();
    expect(heading!.textContent).toBe(formulaTable.th.refusalTitle);

    const table = alert!.querySelector('table.formula-refusal-table');
    expect(table).not.toBeNull();

    const headers = Array.from(table!.querySelectorAll('thead th')).map((th) => th.textContent);
    expect(headers).toEqual([
      formulaTable.th.colBranch,
      formulaTable.th.colCurrency,
      formulaTable.th.colTier,
      formulaTable.th.colMode,
      formulaTable.th.colBuy,
      formulaTable.th.colSell,
      formulaTable.th.colProblem,
    ]);

    const rows = table!.querySelectorAll('tbody tr');
    expect(rows).toHaveLength(2);

    const cells1 = Array.from(rows[0].querySelectorAll('td')).map((td) => td.textContent);
    expect(cells1).toEqual([
      'MAIN',
      'USD',
      formulaTable.th.dash,
      formulaTable.th.modeAuto,
      '33.8837374',
      '33.7293303',
      'failure line 1',
    ]);

    const cells2 = Array.from(rows[1].querySelectorAll('td')).map((td) => td.textContent);
    expect(cells2).toEqual([
      'BRANCH-2',
      'JPY',
      '2',
      formulaTable.th.modeAuto,
      '33.8837374',
      '33.7293303',
      'failure line 2',
    ]);
  });

  it('R2: no lines renders nothing (no role="alert" element)', () => {
    const el = mount(() => <FormulaRefusal lines={() => []} />);
    expect(el.querySelector('[role="alert"]')).toBeNull();
    expect(el.querySelector('.formula-refusal')).toBeNull();
    expect(el.querySelector('table')).toBeNull();
  });

  it('renders mode and dash cells correctly for follow mode and null values', () => {
    const followWithNulls = makeLine({
      follow: true,
      tier: null,
      buy: null,
      sell: null,
      raw: 'follow line',
    });
    const autoWithValues = makeLine({
      follow: false,
      tier: 3,
      buy: '35.00',
      sell: '36.00',
      raw: 'auto line',
    });

    const el = mount(() => <FormulaRefusal lines={() => [followWithNulls, autoWithValues]} />);

    const rows = el.querySelectorAll('tbody tr');
    expect(rows).toHaveLength(2);

    const cells1 = Array.from(rows[0].querySelectorAll('td')).map((td) => td.textContent);
    expect(cells1[2]).toBe(formulaTable.th.dash);
    expect(cells1[3]).toBe(formulaTable.th.modeFollow);
    expect(cells1[4]).toBe(formulaTable.th.dash);
    expect(cells1[5]).toBe(formulaTable.th.dash);

    const cells2 = Array.from(rows[1].querySelectorAll('td')).map((td) => td.textContent);
    expect(cells2[2]).toBe('3');
    expect(cells2[3]).toBe(formulaTable.th.modeAuto);
    expect(cells2[4]).toBe('35.00');
    expect(cells2[5]).toBe('36.00');
  });

  it('switches text to English when language is en', () => {
    setLang('en');
    const line = makeLine({ follow: true });

    const el = mount(() => <FormulaRefusal lines={() => [line]} />);

    const alert = el.querySelector('[role="alert"].formula-refusal');
    expect(alert).not.toBeNull();
    expect(alert!.querySelector('h3')!.textContent).toBe(formulaTable.en.refusalTitle);

    const headers = Array.from(alert!.querySelectorAll('thead th')).map((th) => th.textContent);
    expect(headers).toEqual([
      formulaTable.en.colBranch,
      formulaTable.en.colCurrency,
      formulaTable.en.colTier,
      formulaTable.en.colMode,
      formulaTable.en.colBuy,
      formulaTable.en.colSell,
      formulaTable.en.colProblem,
    ]);

    const cells = Array.from(alert!.querySelectorAll('tbody td')).map((td) => td.textContent);
    expect(cells[3]).toBe(formulaTable.en.modeFollow);
    expect(cells[2]).toBe(formulaTable.en.dash);
  });

  it('updates reactively when lines signal changes', () => {
    const [lines, setLines] = createSignal<SpreadFailureLine[]>([]);

    const el = mount(() => <FormulaRefusal lines={lines} />);
    expect(el.querySelector('[role="alert"]')).toBeNull();

    setLines([makeLine({ branch: 'TEST' })]);
    expect(el.querySelector('[role="alert"]')).not.toBeNull();
    expect(el.querySelectorAll('tbody tr')).toHaveLength(1);
    expect(el.querySelector('tbody td')!.textContent).toBe('TEST');

    setLines([]);
    expect(el.querySelector('[role="alert"]')).toBeNull();
  });
});
