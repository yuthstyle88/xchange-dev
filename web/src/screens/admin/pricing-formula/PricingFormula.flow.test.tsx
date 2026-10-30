// Pricing formula FE flow (docs/design/blueprint/pricing-formula-web.md §3): the organisation screen with the real
// parts (editor, live preview, preview table, refusal, spread-failure parser), only `fetch` faked. The per-ticket tests
// replace the parts with stand-ins; this one proves they fit together. Written by the sub-controller
// claude/studio-claude-3 after wave 4 merged into feat/pricing-formula-web.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeFormulaCheckRowDto,
  makeFormulaNotCheckedDto,
  makeFormulaParseErrorDto,
  makeOrganisationFormulaDto,
  makePricingFormulaData,
  makePricingFormulaPreviewOutput,
} from '../../../shared/testing/fixtures';
import type { AdminMenuProps } from '../menus';
import { formulaT } from '../parts/formula-i18n';
import { PricingFormula } from './PricingFormula';

vi.mock('../parts/toast', () => ({ toast: vi.fn(), Toast: () => null }));

const VIEW = '/api/v3/views/pricing_formula';
const PREVIEW = '/api/pricing/formula/preview';
const SAVE = '/api/pricing/formula';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const VIEW_DATA = makePricingFormulaData({
  organisation: makeOrganisationFormulaDto({
    version: 3,
    buyFormula: 'ref_mid - 0.05',
    sellFormula: 'ref_mid + 0.05',
    validFrom: '2026-10-29T10:00:00Z',
  }),
  default: {
    buyFormula: 'ref_buy * (1 + (buy_pct + tier_buy_pct) / 100)',
    sellFormula: 'ref_sell * (1 + (sell_pct + tier_sell_pct) / 100)',
  },
  inputs: [{ name: 'ref_buy' }, { name: 'ref_sell' }, { name: 'ref_mid' }],
  branches: [{ branch: 'MAIN', source: 'ORGANISATION', own: null }],
});

const PREVIEW_OK = makePricingFormulaPreviewOutput({
  parseError: null,
  rows: [
    makeFormulaCheckRowDto({
      branch: 'MAIN',
      currency: 'JPY',
      unitSize: '100',
      mode: 'AUTO',
      buy: '21.5',
      sell: '22.1',
    }),
    makeFormulaCheckRowDto({
      branch: 'MAIN',
      currency: 'USD',
      unitSize: '1',
      mode: 'AUTO',
      buy: '34.1',
      sell: '33.9',
      failure: 'MAIN USD buy 34.1 > sell 33.9',
    }),
  ],
  notChecked: [makeFormulaNotCheckedDto({ branch: 'B2', currency: 'EUR' })],
  ok: false,
});

describe('pricing-formula-web flow: the organisation screen with its real parts', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let previewAnswer: () => Response;
  let saveAnswer: () => Response;
  let previews: Record<string, unknown>[];
  let saves: Record<string, unknown>[];

  beforeEach(() => {
    setLang('th');
    previews = [];
    saves = [];
    previewAnswer = () => json(PREVIEW_OK);
    saveAnswer = () => json({ id: 'f-4', version: 4, notChecked: [] });
    host = document.createElement('div');
    document.body.appendChild(host);
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.startsWith(VIEW)) return json({ version: 1, changed: true, data: VIEW_DATA });
        if (url.startsWith(PREVIEW)) {
          previews.push(JSON.parse(String(init?.body ?? '{}')));
          return previewAnswer();
        }
        if (url.startsWith(SAVE)) {
          saves.push(JSON.parse(String(init?.body ?? '{}')));
          return saveAnswer();
        }
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

  function mount(): void {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(() => <PricingFormula {...props} />, host);
  }

  const q = <T extends Element>(sel: string) => host.querySelector<T>(sel);
  const all = (sel: string) => [...host.querySelectorAll(sel)];
  const flat = (el: Element | null) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

  function type(sel: string, value: string): void {
    const el = q<HTMLTextAreaElement>(sel);
    expect(el).not.toBeNull();
    el!.value = value;
    el!.dispatchEvent(new Event('input', { bubbles: true }));
  }

  it('loads, previews the organisation formula per branch and currency with the unit, the failure and notChecked', async () => {
    mount();
    await vi.waitFor(() => expect(previews).toHaveLength(1), { timeout: 3000 });
    expect(previews[0]).toMatchObject({
      branch: 'MAIN',
      buyFormula: 'ref_mid - 0.05',
      sellFormula: 'ref_mid + 0.05',
      scope: 'ORGANISATION',
    });
    await vi.waitFor(() => expect(all('.formula-preview-table tbody tr')).toHaveLength(2));
    const [jpy, usd] = all('.formula-preview-table tbody tr');
    expect(flat(jpy!)).toContain(formulaT('unitPer', { unit: '100', currency: 'JPY' }));
    expect(flat(jpy!)).toContain(formulaT('previewOk'));
    expect(usd!.classList.contains('formula-row--fail')).toBe(true);
    expect(flat(usd!)).toContain(formulaT('buyAboveSell', { buy: '34.1', sell: '33.9' }));
    expect(flat(host)).toContain(formulaT('unitNote'));
    expect(flat(host)).toContain('B2 EUR');
  });

  it('a parse error stops the table and shows the code under its field', async () => {
    previewAnswer = () =>
      json(
        makePricingFormulaPreviewOutput({
          parseError: makeFormulaParseErrorDto({ field: 'sellFormula', code: 'unknownInput', message: 'ref_foo' }),
          rows: [],
          notChecked: [],
          ok: false,
        }),
      );
    mount();
    await vi.waitFor(() => expect(q('[data-field-error="sellFormula"]')).not.toBeNull(), { timeout: 3000 });
    expect(flat(q('[data-field-error="sellFormula"]'))).toContain(formulaT('code.unknownInput'));
    expect(q('.formula-preview-table')).toBeNull();
    expect(flat(host)).toContain(formulaT('previewFixFirst'));
  });

  it('a refused save lists every failing branch, currency and tier from the server lines', async () => {
    saveAnswer = () =>
      json(
        {
          error: {
            code: 'validationFailed',
            field: 'buyFormula',
            message: 'MAIN USD buy 34.1 > sell 33.9\nB2 JPY tier 2 FOLLOW divideByZero',
          },
        },
        400,
      );
    mount();
    await vi.waitFor(() => expect(q('textarea[data-field="reason"]')).not.toBeNull());
    type('textarea[data-field="reason"]', 'narrow the spread');
    q<HTMLButtonElement>('button[data-action="formula-save"]')!.click();
    await vi.waitFor(() => expect(all('.formula-refusal-table tbody tr')).toHaveLength(2));
    expect(saves[0]).toMatchObject({ branch: 'MAIN', reason: 'narrow the spread' });
    expect(q('.formula-refusal')!.getAttribute('role')).toBe('alert');
    const [usd, jpy] = all('.formula-refusal-table tbody tr');
    expect(flat(usd!)).toContain('USD');
    expect(flat(usd!)).toContain(formulaT('buyAboveSell', { buy: '34.1', sell: '33.9' }));
    expect(flat(jpy!)).toContain('B2');
    expect(flat(jpy!)).toContain('2');
    expect(flat(jpy!)).toContain(formulaT('modeFollow'));
    expect(flat(jpy!)).toContain(formulaT('code.divideByZero'));
  });
});
