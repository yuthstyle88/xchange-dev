// Pricing formula FE T9 up-front tests (docs/design/blueprint/pricing-formula-web.md §3 C1–C7, §3a DOM hooks),
// written before the implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken; add
// tests in a file of your own. The override block, `formula-override.ts`, the refusal list and the failure parser are
// replaced by stand-ins so this ticket does not wait for T2, T5, T7 or T8. `RatesPolicyCardProps` does not change.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import {
  makeRatePolicyData,
  makeRatePolicyVersionDto,
  type RatePolicySaveInput,
  type RatePolicySaveOutput,
} from '../../../shared/testing/fixtures';
import type { RatesFormulaOverrideProps } from './RatesFormulaOverride';
import { RatesPolicyCard } from './RatesPolicyCard';
import * as ratesTable from './i18n';

const { overrideProps } = vi.hoisted(() => ({ overrideProps: [] as RatesFormulaOverrideProps[] }));

vi.mock('./RatesFormulaOverride', () => ({
  RatesFormulaOverride: (p: RatesFormulaOverrideProps) => {
    overrideProps.push(p);
    return null;
  },
}));
vi.mock('./formula-override', () => {
  const EMPTY = { current: null, choice: 'ORGANISATION', buy: '', sell: '' };
  type S = { current: unknown; choice: string; buy: string; sell: string };
  return {
    EMPTY_OVERRIDE: EMPTY,
    initialOverride: () => EMPTY,
    overridePayload: (s: S) =>
      s.current === null || s.choice !== 'BRANCH' ? {} : { buyFormula: s.buy.trim(), sellFormula: s.sell.trim() },
    overrideBlank: (s: S) =>
      s.current !== null && s.choice === 'BRANCH' && s.buy.trim() === ''
        ? 'buyFormula'
        : s.current !== null && s.choice === 'BRANCH' && s.sell.trim() === ''
          ? 'sellFormula'
          : null,
  };
});
vi.mock('../parts/FormulaRefusal', () => import('../parts/formula-stubs.testing'));
vi.mock('../parts/spread-failure', () => import('../parts/formula-stubs.testing'));

const OWN_ENTRY = { branch: 'MAIN', source: 'ORGANISATION', own: null };

describe('pricing-formula-web T9: the policy card saves the branch formula', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: (RatePolicySaveInput & { branch: string })[];
  let answer: () => Promise<RatePolicySaveOutput>;
  let toasts: string[];
  const [branch, setBranch] = createSignal('MAIN');

  beforeEach(() => {
    setLang('th');
    overrideProps.length = 0;
    setBranch('MAIN');
    writes = [];
    toasts = [];
    answer = async () => ({ id: 'pol-2', version: 2, notChecked: [] });
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  function mount(): void {
    const [policyData] = createSignal(
      makeRatePolicyData({
        policy: makeRatePolicyVersionDto({
          id: 'pol-1',
          branchId: 'MAIN',
          version: 1,
          buyAdjustPct: '0.3000',
          sellAdjustPct: '-0.3000',
          guards: {},
          maxReferenceAgeHours: 24,
          onStale: 'STOP',
          staleLimitThb: null,
          reason: 'Initial',
          valueTiersOn: false,
          valueTiers: [],
          sharpMoveAlertPct: null,
        }),
      }),
    );
    const [valueTiers, setValueTiers] = createSignal(
      ([1, 2, 3, 4] as const).map((tierNo) => ({ tierNo, upperInclusiveThb: '', buyMarginPct: '', sellMarginPct: '' })),
    );
    const [valueTiersOn, setValueTiersOn] = createSignal(false);
    const [sharp, setSharp] = createSignal('');
    dispose = render(
      () => (
        <RatesPolicyCard
          branch={branch}
          busy={() => false}
          canSavePolicy={() => true}
          policyData={policyData}
          policyError={() => null}
          valueTiers={valueTiers}
          setValueTiers={setValueTiers}
          valueTiersOn={valueTiersOn}
          setValueTiersOn={setValueTiersOn}
          sharpMoveAlertPct={sharp}
          setSharpMoveAlertPct={setSharp}
          adjustBuyPct={() => '0.3000'}
          adjustSellPct={() => '-0.3000'}
          savePolicyWrite={{
            async send(body) {
              writes.push(body);
              return answer();
            },
            reset() {},
          }}
          readGen={() => 1}
          setSimulateResult={() => {}}
          guarded={(fn) => fn}
          toast={(m) => toasts.push(m)}
          t={(k) => ratesTable.th[k]}
        />
      ),
      host,
    );
  }

  const q = <T extends Element>(sel: string) => host.querySelector<T>(sel);
  const block = () => overrideProps.at(-1)!;

  async function save(): Promise<void> {
    await vi.waitFor(() => expect(q('button[data-testid="save-policy"]')).not.toBeNull());
    q<HTMLButtonElement>('button[data-testid="save-policy"]')!.click();
    await new Promise((r) => setTimeout(r, 20));
  }

  it('the card mounts the override block with this branch, an empty state and a reload key', async () => {
    mount();
    await vi.waitFor(() => expect(overrideProps.length).toBeGreaterThan(0));
    expect(block().branch()).toBe('MAIN');
    expect(block().state().current).toBeNull();
    expect(typeof block().reloadKey()).toBe('number');
  });

  it('C1: block never opened ⇒ the body has no formula key', async () => {
    mount();
    await save();
    expect(writes).toHaveLength(1);
    expect('buyFormula' in writes[0]).toBe(false);
    expect('sellFormula' in writes[0]).toBe(false);
  });

  it('C2: an own formula typed ⇒ both texts in the body', async () => {
    mount();
    await vi.waitFor(() => expect(overrideProps.length).toBeGreaterThan(0));
    block().setState({ current: OWN_ENTRY, choice: 'BRANCH', buy: ' ref_mid - 0.05 ', sell: 'ref_mid + 0.05' });
    await save();
    expect(writes).toHaveLength(1);
    expect(writes[0]).toMatchObject({ buyFormula: 'ref_mid - 0.05', sellFormula: 'ref_mid + 0.05' });
    expect(writes[0]).toMatchObject({ buyAdjustPct: '0.3000', sellAdjustPct: '-0.3000' });
  });

  it('C3: BRANCH with a blank text ⇒ nothing sent, a formula field error', async () => {
    mount();
    await vi.waitFor(() => expect(overrideProps.length).toBeGreaterThan(0));
    block().setState({ current: OWN_ENTRY, choice: 'BRANCH', buy: 'ref_buy', sell: '  ' });
    await save();
    expect(writes).toHaveLength(0);
    await vi.waitFor(() => expect(q('[data-field-error="formula"]')).not.toBeNull());
  });

  it('C4: a refused check shows every failing line and toasts that the save was refused', async () => {
    answer = async () => {
      throw new ApiError({
        status: 400,
        code: 'validationFailed',
        field: 'buyAdjustPct',
        message: 'MAIN USD buy 33.88 > sell 33.72\nMAIN JPY tier 3 buy 0.3 > sell 0.2',
      });
    };
    mount();
    await save();
    await vi.waitFor(() => expect(host.querySelectorAll('[data-stub="refusal"] li')).toHaveLength(2));
    expect(q('[data-stub="refusal"]')!.textContent).toContain('MAIN JPY tier 3');
    expect(toasts.some((m) => m.includes('บันทึกนโยบายไม่ได้'))).toBe(true);
  });

  it('C5: an ordinary refusal keeps today’s path, no refusal list', async () => {
    answer = async () => {
      throw new ApiError({ status: 400, code: 'validationFailed', field: 'buyAdjustPct', message: 'out of range' });
    };
    mount();
    await save();
    expect(q('[data-stub="refusal"]')).toBeNull();
    expect(toasts).toHaveLength(1);
  });

  it('C6: a successful save clears the refusal, moves the reload key and names notChecked in the toast', async () => {
    answer = async () => {
      throw new ApiError({ status: 400, code: 'validationFailed', field: 'buyFormula', message: 'MAIN USD buy 2 > sell 1' });
    };
    mount();
    await save();
    await vi.waitFor(() => expect(q('[data-stub="refusal"]')).not.toBeNull());
    const before = block().reloadKey();
    answer = async () => ({ id: 'pol-2', version: 2, notChecked: [{ branch: 'MAIN', currency: 'KRW' }] });
    await save();
    await vi.waitFor(() => expect(q('[data-stub="refusal"]')).toBeNull());
    expect(block().reloadKey()).not.toBe(before);
    expect(toasts.some((m) => m.includes('KRW'))).toBe(true);
  });

  it('C7: a branch change clears the refusal', async () => {
    answer = async () => {
      throw new ApiError({ status: 400, code: 'validationFailed', field: 'buyFormula', message: 'MAIN USD buy 2 > sell 1' });
    };
    mount();
    await save();
    await vi.waitFor(() => expect(q('[data-stub="refusal"]')).not.toBeNull());
    setBranch('B2');
    await vi.waitFor(() => expect(q('[data-stub="refusal"]')).toBeNull());
  });
});
