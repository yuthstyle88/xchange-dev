// Pricing formula FE T4 tests (docs/design/blueprint/pricing-formula-web.md §3 T1r–T5r).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { PricingFormulaPreviewOutput } from '../../../shared/api/generated/pricing_formula_preview';
import { en, th } from './formula-i18n';
import { FormulaPreviewTable } from './FormulaPreviewTable';

vi.mock('./spread-failure', () => import('./formula-stubs.testing'));

describe('FormulaPreviewTable (T1r–T5r)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    setLang('th');
  });

  it('T1r: renders table columns branch, currency, unit, tier, mode, buy, sell, result, and failing-row class', () => {
    const output: PricingFormulaPreviewOutput = {
      parseError: null,
      ok: false,
      rows: [
        {
          branch: 'B2',
          currency: 'JPY',
          tier: 1,
          mode: 'AUTO',
          unitSize: '100',
          buy: '23.45',
          sell: '23.65',
          failure: null,
        },
        {
          branch: 'MAIN',
          currency: 'USD',
          tier: null,
          mode: 'FOLLOW',
          unitSize: '1',
          buy: '34.12',
          sell: null,
          failure: null,
        },
        {
          branch: 'MAIN',
          currency: 'USD',
          tier: 2,
          mode: 'AUTO',
          unitSize: '1',
          buy: '35.00',
          sell: '34.00',
          failure: 'MAIN USD tier 2 buy 35.00 > sell 34.00',
        },
        {
          branch: 'MAIN',
          currency: 'EUR',
          tier: null,
          mode: 'AUTO',
          unitSize: '1',
          buy: '38.00',
          sell: '38.50',
          failure: 'unknown custom failure',
        },
      ],
      notChecked: [],
    };

    dispose = render(
      () => (
        <FormulaPreviewTable
          result={() => output}
          pending={() => false}
          error={() => null}
        />
      ),
      host,
    );

    const tableEl = host.querySelector<HTMLTableElement>('table.formula-preview-table');
    expect(tableEl).not.toBeNull();

    // Headers
    const headers = host.querySelectorAll('th');
    expect(headers.length).toBe(8);
    expect(headers[0].textContent).toBe(th.colBranch);
    expect(headers[1].textContent).toBe(th.colCurrency);
    expect(headers[2].textContent).toBe(th.colUnit);
    expect(headers[3].textContent).toBe(th.colTier);
    expect(headers[4].textContent).toBe(th.colMode);
    expect(headers[5].textContent).toBe(th.colBuy);
    expect(headers[6].textContent).toBe(th.colSell);
    expect(headers[7].textContent).toBe(th.colResult);

    // Row 1: B2 JPY tier 1 AUTO, passes
    const row1 = host.querySelector<HTMLTableRowElement>('tr[data-row="B2:JPY:1"]');
    expect(row1).not.toBeNull();
    expect(row1!.classList.contains('formula-row--fail')).toBe(false);
    const cells1 = row1!.querySelectorAll('td');
    expect(cells1[0].textContent).toBe('B2');
    expect(cells1[1].textContent).toBe('JPY');
    expect(cells1[2].textContent).toBe('ต่อ 100 JPY');
    expect(cells1[3].textContent).toBe('1');
    expect(cells1[4].textContent).toBe(th.modeAuto);
    expect(cells1[5].textContent).toBe('23.45');
    expect(cells1[6].textContent).toBe('23.65');
    expect(cells1[7].textContent).toBe(th.previewOk);

    // Row 2: MAIN USD single FOLLOW, sell is null (dash)
    const row2 = host.querySelector<HTMLTableRowElement>('tr[data-row="MAIN:USD:single"]');
    expect(row2).not.toBeNull();
    expect(row2!.classList.contains('formula-row--fail')).toBe(false);
    const cells2 = row2!.querySelectorAll('td');
    expect(cells2[0].textContent).toBe('MAIN');
    expect(cells2[1].textContent).toBe('USD');
    expect(cells2[2].textContent).toBe('ต่อ 1 USD');
    expect(cells2[3].textContent).toBe(th.dash);
    expect(cells2[4].textContent).toBe(th.modeFollow);
    expect(cells2[5].textContent).toBe('34.12');
    expect(cells2[6].textContent).toBe(th.dash);
    expect(cells2[7].textContent).toBe(th.previewOk);

    // Row 3: MAIN USD tier 2 failing (parsed failure)
    const row3 = host.querySelector<HTMLTableRowElement>('tr[data-row="MAIN:USD:2"]');
    expect(row3).not.toBeNull();
    expect(row3!.classList.contains('formula-row--fail')).toBe(true);
    const cells3 = row3!.querySelectorAll('td');
    expect(cells3[7].textContent).toBe('MAIN USD tier 2 buy 35.00 > sell 34.00');

    // Row 4: unparsed failure shows as received
    const row4 = host.querySelector<HTMLTableRowElement>('tr[data-row="MAIN:EUR:single"]');
    expect(row4).not.toBeNull();
    expect(row4!.classList.contains('formula-row--fail')).toBe(true);
    const cells4 = row4!.querySelectorAll('td');
    expect(cells4[7].textContent).toBe('unknown custom failure');
  });

  it('T2r: renders unitNote under table whenever rows show', () => {
    const withRows: PricingFormulaPreviewOutput = {
      parseError: null,
      ok: true,
      rows: [
        {
          branch: 'MAIN',
          currency: 'USD',
          tier: null,
          mode: 'AUTO',
          unitSize: '1',
          buy: '34.00',
          sell: '34.50',
          failure: null,
        },
      ],
      notChecked: [],
    };

    const [result, setResult] = createSignal<PricingFormulaPreviewOutput | null>(withRows);

    dispose = render(
      () => (
        <FormulaPreviewTable
          result={result}
          pending={() => false}
          error={() => null}
        />
      ),
      host,
    );

    const note = host.querySelector<HTMLDivElement>('.formula-unit-note');
    expect(note).not.toBeNull();
    expect(note!.textContent).toBe(th.unitNote);

    // When rows are empty, unitNote does not show
    setResult({ parseError: null, ok: true, rows: [], notChecked: [] });
    expect(host.querySelector('.formula-unit-note')).toBeNull();
  });

  it('T3r: notChecked non-empty renders heading and list items', () => {
    const output: PricingFormulaPreviewOutput = {
      parseError: null,
      ok: true,
      rows: [],
      notChecked: [
        { branch: 'MAIN', currency: 'CHF' },
        { branch: 'B2', currency: 'SGD' },
      ],
    };

    dispose = render(
      () => (
        <FormulaPreviewTable
          result={() => output}
          pending={() => false}
          error={() => null}
        />
      ),
      host,
    );

    expect(host.textContent).toContain(th.notCheckedTitle);
    const list = host.querySelector<HTMLUListElement>('ul[data-testid="formula-preview-not-checked"]');
    expect(list).not.toBeNull();
    expect(list!.classList.contains('formula-not-checked')).toBe(true);

    const items = list!.querySelectorAll('li');
    expect(items.length).toBe(2);
    expect(items[0].textContent).toBe('MAIN CHF');
    expect(items[1].textContent).toBe('B2 SGD');
  });

  it('T4r: pending() shows previewPending, error() shows form-error, null result shows nothing', () => {
    const [pending, setPending] = createSignal(true);
    const [error, setError] = createSignal<string | null>(null);
    const [result] = createSignal<PricingFormulaPreviewOutput | null>(null);

    dispose = render(
      () => (
        <FormulaPreviewTable
          result={result}
          pending={pending}
          error={error}
        />
      ),
      host,
    );

    // pending() => previewPending
    const pendingEl = host.querySelector<HTMLDivElement>('.formula-pending');
    expect(pendingEl).not.toBeNull();
    expect(pendingEl!.textContent).toBe(th.previewPending);
    expect(host.querySelector('table')).toBeNull();

    // error() => form-error
    setPending(false);
    setError('Network timeout');
    const errorEl = host.querySelector<HTMLDivElement>('.form-error');
    expect(errorEl).not.toBeNull();
    expect(errorEl!.getAttribute('role')).toBe('alert');
    expect(errorEl!.textContent).toBe('Network timeout');
    expect(host.querySelector('table')).toBeNull();

    // result() null and neither pending nor error => nothing
    setError(null);
    expect(host.querySelector('.formula-preview')).toBeNull();
    expect(host.querySelector('table')).toBeNull();
  });

  it('T5r: parseError set renders previewFixFirst and no table', () => {
    const output: PricingFormulaPreviewOutput = {
      parseError: {
        field: 'buyFormula',
        code: 'syntax',
        message: 'unexpected token',
      },
      ok: false,
      rows: [
        {
          branch: 'MAIN',
          currency: 'USD',
          tier: null,
          mode: 'AUTO',
          unitSize: '1',
          buy: '34.00',
          sell: '34.50',
          failure: null,
        },
      ],
      notChecked: [],
    };

    dispose = render(
      () => (
        <FormulaPreviewTable
          result={() => output}
          pending={() => false}
          error={() => null}
        />
      ),
      host,
    );

    expect(host.querySelector('table')).toBeNull();
    expect(host.textContent).toContain(th.previewFixFirst);
  });

  it('supports English mode without hardcoded Thai text', () => {
    setLang('en');
    const output: PricingFormulaPreviewOutput = {
      parseError: null,
      ok: true,
      rows: [
        {
          branch: 'MAIN',
          currency: 'USD',
          tier: null,
          mode: 'AUTO',
          unitSize: '1',
          buy: '34.00',
          sell: '34.50',
          failure: null,
        },
      ],
      notChecked: [{ branch: 'B2', currency: 'EUR' }],
    };

    dispose = render(
      () => (
        <FormulaPreviewTable
          result={() => output}
          pending={() => false}
          error={() => null}
        />
      ),
      host,
    );

    const headers = host.querySelectorAll('th');
    expect(headers[0].textContent).toBe(en.colBranch);
    expect(headers[1].textContent).toBe(en.colCurrency);
    expect(headers[2].textContent).toBe(en.colUnit);
    expect(headers[7].textContent).toBe(en.colResult);

    const row = host.querySelector<HTMLTableRowElement>('tr[data-row="MAIN:USD:single"]')!;
    const cells = row.querySelectorAll('td');
    expect(cells[2].textContent).toBe('per 1 USD');
    expect(cells[4].textContent).toBe(en.modeAuto);
    expect(cells[7].textContent).toBe(en.previewOk);

    expect(host.querySelector('.formula-unit-note')?.textContent).toBe(en.unitNote);
    expect(host.textContent).toContain(en.notCheckedTitle);
    expect(host.textContent).toContain(en.previewTitle);
  });
});
