// Pricing formula FE T7 up-front tests (docs/design/blueprint/pricing-formula-web.md §3 V1–V8, §3a DOM hooks),
// written before the implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken; add
// tests in a file of your own. The shared parts and `formula-override.ts` are replaced by stand-ins so this ticket
// does not wait for T1–T5 or T8.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeOrganisationFormulaDto, makePricingFormulaData, type BranchFormulaDto } from '../../../shared/testing/fixtures';
import { previewCalls, resetFormulaStubs } from '../parts/formula-stubs.testing';
import { EMPTY_OVERRIDE, type FormulaOverrideState } from './formula-override';
import { RatesFormulaOverride } from './RatesFormulaOverride';

vi.mock('../parts/FormulaEditor', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/FormulaPreviewTable', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/formula-preview', () => import('../parts/formula-stubs.testing'));
vi.mock('./formula-override', () => {
  const EMPTY = { current: null, choice: 'ORGANISATION', buy: '', sell: '' };
  return {
    EMPTY_OVERRIDE: EMPTY,
    initialOverride: (entry: BranchFormulaDto, org: { buyFormula: string; sellFormula: string }) =>
      entry.source === 'BRANCH' && entry.own
        ? { current: entry, choice: 'BRANCH', buy: entry.own.buyFormula, sell: entry.own.sellFormula }
        : { current: entry, choice: 'ORGANISATION', buy: org.buyFormula, sell: org.sellFormula },
    overridePayload: () => ({}),
    overrideBlank: () => null,
  };
});

const VIEW = '/api/v3/views/pricing_formula';
const ORG = { buyFormula: 'ref_mid - 0.05', sellFormula: 'ref_mid + 0.05' };
const DEFAULT = {
  buyFormula: 'ref_buy * (1 + (buy_pct + tier_buy_pct) / 100)',
  sellFormula: 'ref_sell * (1 + (sell_pct + tier_sell_pct) / 100)',
};

function data(branches: BranchFormulaDto[], organisation = true) {
  return makePricingFormulaData({
    organisation: organisation
      ? makeOrganisationFormulaDto({
          id: 'f-3',
          version: 3,
          ...ORG,
          validFrom: '2026-10-29T10:00:00Z',
          approvedBy: 'exec-1',
          reason: 'narrow',
        })
      : null,
    default: DEFAULT,
    inputs: [{ name: 'ref_buy' }, { name: 'ref_sell' }, { name: 'ref_mid' }],
    branches,
  });
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

describe('pricing-formula-web T7: the branch override block', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let reads: string[];
  let viewAnswer: () => Response;
  const [branch, setBranch] = createSignal('MAIN');
  const [state, setStateSignal] = createSignal<FormulaOverrideState>(EMPTY_OVERRIDE);
  const [reloadKey, setReloadKey] = createSignal(0);
  const setState = vi.fn((s: FormulaOverrideState) => setStateSignal(s));

  beforeEach(() => {
    setLang('th');
    resetFormulaStubs();
    setBranch('MAIN');
    setStateSignal(EMPTY_OVERRIDE);
    setReloadKey(0);
    setState.mockClear();
    reads = [];
    viewAnswer = () =>
      json({
        version: 1,
        changed: true,
        data: data([
          { branch: 'MAIN', source: 'ORGANISATION', own: null },
          { branch: 'B2', source: 'BRANCH', own: { buyFormula: 'ref_buy * 0.99', sellFormula: 'ref_sell * 1.01' } },
        ]),
      });
    host = document.createElement('div');
    document.body.appendChild(host);
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.startsWith(VIEW)) {
          reads.push(url);
          return viewAnswer();
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
    dispose = render(
      () => (
        <RatesFormulaOverride
          branch={branch}
          state={state}
          setState={setState}
          reloadKey={reloadKey}
          disabled={() => false}
        />
      ),
      host,
    );
  }

  const q = <T extends Element>(sel: string) => host.querySelector<T>(sel);
  const text = (sel: string) => (q(sel)?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const radio = (v: string) => q<HTMLInputElement>(`input[type="radio"][name="formula-choice"][value="${v}"]`);

  async function open(): Promise<void> {
    const button = q<HTMLButtonElement>('button[data-action="formula-override-open"]');
    expect(button).not.toBeNull();
    button!.click();
    await vi.waitFor(() => expect(setState).toHaveBeenCalled());
  }

  it('V1: closed ⇒ no read and the state stays empty', async () => {
    mount();
    await new Promise((r) => setTimeout(r, 20));
    expect(reads).toHaveLength(0);
    expect(setState).not.toHaveBeenCalled();
    expect(q('[data-stub="formula-editor"]')).toBeNull();
  });

  it('V2: open reads the view once for this branch and initialises from this branch’s entry', async () => {
    mount();
    await open();
    expect(reads).toHaveLength(1);
    expect(reads[0]).toContain('branch=MAIN');
    expect(setState).toHaveBeenLastCalledWith({
      current: { branch: 'MAIN', source: 'ORGANISATION', own: null },
      choice: 'ORGANISATION',
      buy: ORG.buyFormula,
      sell: ORG.sellFormula,
    });
  });

  it('V2: no organisation formula ⇒ prefilled from the default texts', async () => {
    viewAnswer = () => json({ version: 1, changed: true, data: data([{ branch: 'MAIN', source: 'DEFAULT', own: null }], false) });
    mount();
    await open();
    expect(setState).toHaveBeenLastCalledWith(expect.objectContaining({ buy: DEFAULT.buyFormula, sell: DEFAULT.sellFormula }));
  });

  it('V3: following the organisation shows its text read-only, with no editor and no preview', async () => {
    mount();
    await open();
    await vi.waitFor(() => expect(radio('ORGANISATION')?.checked).toBe(true));
    expect(text('[data-testid="formula-override-source"]')).toContain('สูตรขององค์กร');
    expect(text('[data-testid="formula-override-org"]')).toContain(ORG.buyFormula);
    expect(text('[data-testid="formula-override-org"]')).toContain(ORG.sellFormula);
    expect(q('[data-stub="formula-editor"]')).toBeNull();
    expect(previewCalls.every((o) => !o.enabled())).toBe(true);
  });

  it('V4: choosing the branch’s own formula shows the editor, the BRANCH preview and the note', async () => {
    mount();
    await open();
    await vi.waitFor(() => expect(radio('BRANCH')).not.toBeNull());
    radio('BRANCH')!.click();
    await vi.waitFor(() => expect(state().choice).toBe('BRANCH'));
    await vi.waitFor(() => expect(q('[data-stub="formula-editor"]')).not.toBeNull());
    const o = previewCalls.at(-1)!;
    expect(o.scope).toBe('BRANCH');
    expect(o.branch()).toBe('MAIN');
    await vi.waitFor(() => expect(o.enabled()).toBe(true));
    expect(o.buy()).toBe(ORG.buyFormula);
    expect(text('[data-testid="formula-override-note"]')).toContain('ที่บันทึกไว้แล้วของสาขา');
  });

  it('V5: an edit sets the state with the new text and choice BRANCH', async () => {
    setBranch('B2');
    mount();
    await open();
    await vi.waitFor(() => expect(q('textarea[data-field="sellFormula"]')).not.toBeNull());
    const sell = q<HTMLTextAreaElement>('textarea[data-field="sellFormula"]')!;
    sell.value = 'ref_sell * 1.02';
    sell.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() =>
      expect(setState).toHaveBeenLastCalledWith(
        expect.objectContaining({ choice: 'BRANCH', buy: 'ref_buy * 0.99', sell: 'ref_sell * 1.02' }),
      ),
    );
  });

  it('V6: a reloadKey change while open reads again and re-initialises', async () => {
    mount();
    await open();
    const calls = setState.mock.calls.length;
    setReloadKey(1);
    await vi.waitFor(() => expect(reads).toHaveLength(2));
    await vi.waitFor(() => expect(setState.mock.calls.length).toBeGreaterThan(calls));
  });

  it('V7: a branch change closes the block and empties the state', async () => {
    mount();
    await open();
    setBranch('B2');
    await vi.waitFor(() => expect(setState).toHaveBeenLastCalledWith(EMPTY_OVERRIDE));
    expect(q('[data-stub="formula-editor"]')).toBeNull();
    expect(radio('ORGANISATION')).toBeNull();
  });

  it('V8: a failed read is worded and the state keeps current null', async () => {
    viewAnswer = () => json({ error: { code: 'serverError', message: 'boom' } }, 500);
    mount();
    q<HTMLButtonElement>('button[data-action="formula-override-open"]')!.click();
    await vi.waitFor(() => expect(q('[data-testid="formula-override-error"]')).not.toBeNull());
    expect(state().current).toBeNull();
    expect(setState.mock.calls.every(([s]) => s.current === null)).toBe(true);
  });
});
