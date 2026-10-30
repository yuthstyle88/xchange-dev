// Pricing formula FE T8 up-front tests (docs/design/blueprint/pricing-formula-web.md §3 O1–O8), written before the
// implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken; add tests in a file of
// your own. Wire meaning: `ratePolicySave` buyFormula/sellFormula absent = keep, null = follow, text = own
// (api/src/presentation/http/v3/commands/rate_policy_save.rs:74-82, :292-320).
import { describe, expect, it } from 'vitest';
import { makeBranchFormulaDto } from '../../../shared/testing/fixtures';
import {
  EMPTY_OVERRIDE,
  initialOverride,
  overrideBlank,
  overridePayload,
  type FormulaOverrideState,
} from './formula-override';

const OWN = makeBranchFormulaDto({
  branch: 'MAIN',
  source: 'BRANCH',
  own: { buyFormula: 'ref_mid - 0.05', sellFormula: 'ref_mid + 0.05' },
});
const FOLLOWS_ORG = makeBranchFormulaDto({ branch: 'MAIN', source: 'ORGANISATION', own: null });
const FOLLOWS_DEFAULT = makeBranchFormulaDto({ branch: 'MAIN', source: 'DEFAULT', own: null });
const ORG = { buyFormula: 'ref_buy * 0.997', sellFormula: 'ref_sell * 1.003' };

function state(over: Partial<FormulaOverrideState>): FormulaOverrideState {
  return { ...EMPTY_OVERRIDE, ...over };
}

describe('pricing-formula-web T8: overridePayload', () => {
  it('O1: never loaded ⇒ keep (no key at all)', () => {
    expect(overridePayload(EMPTY_OVERRIDE)).toEqual({});
    expect(overridePayload(state({ choice: 'BRANCH', buy: 'ref_buy', sell: 'ref_sell' }))).toEqual({});
    expect('buyFormula' in overridePayload(EMPTY_OVERRIDE)).toBe(false);
  });

  it('O2: follow the organisation while the branch has its own ⇒ both null', () => {
    expect(overridePayload(state({ current: OWN, choice: 'ORGANISATION', buy: 'x', sell: 'y' }))).toEqual({
      buyFormula: null,
      sellFormula: null,
    });
  });

  it('O3: follow the organisation while already following ⇒ keep', () => {
    expect(overridePayload(state({ current: FOLLOWS_ORG, choice: 'ORGANISATION' }))).toEqual({});
    expect(overridePayload(state({ current: FOLLOWS_DEFAULT, choice: 'ORGANISATION' }))).toEqual({});
  });

  it('O4: own texts unchanged (after trimming) ⇒ keep', () => {
    expect(
      overridePayload(state({ current: OWN, choice: 'BRANCH', buy: '  ref_mid - 0.05 ', sell: 'ref_mid + 0.05\n' })),
    ).toEqual({});
  });

  it('O5: own texts changed or newly set ⇒ both texts, trimmed', () => {
    expect(
      overridePayload(state({ current: OWN, choice: 'BRANCH', buy: ' ref_mid - 0.04 ', sell: 'ref_mid + 0.05' })),
    ).toEqual({ buyFormula: 'ref_mid - 0.04', sellFormula: 'ref_mid + 0.05' });
    expect(
      overridePayload(state({ current: FOLLOWS_ORG, choice: 'BRANCH', buy: 'ref_buy', sell: ' ref_sell ' })),
    ).toEqual({ buyFormula: 'ref_buy', sellFormula: 'ref_sell' });
  });
});

describe('pricing-formula-web T8: overrideBlank', () => {
  it('O6: a blank text under BRANCH names the field, buy first', () => {
    expect(overrideBlank(state({ current: FOLLOWS_ORG, choice: 'BRANCH', buy: '  ', sell: '' }))).toBe('buyFormula');
    expect(overrideBlank(state({ current: FOLLOWS_ORG, choice: 'BRANCH', buy: 'ref_buy', sell: ' ' }))).toBe(
      'sellFormula',
    );
  });

  it('O6: nothing blank, or not BRANCH ⇒ null', () => {
    expect(overrideBlank(state({ current: FOLLOWS_ORG, choice: 'BRANCH', buy: 'ref_buy', sell: 'ref_sell' }))).toBeNull();
    expect(overrideBlank(state({ current: OWN, choice: 'ORGANISATION', buy: '', sell: '' }))).toBeNull();
    expect(overrideBlank(EMPTY_OVERRIDE)).toBeNull();
  });
});

describe('pricing-formula-web T8: initialOverride', () => {
  it('O7: the branch has its own ⇒ BRANCH with those texts', () => {
    expect(initialOverride(OWN, ORG)).toEqual({
      current: OWN,
      choice: 'BRANCH',
      buy: 'ref_mid - 0.05',
      sell: 'ref_mid + 0.05',
    });
  });

  it('O8: the branch follows ⇒ ORGANISATION with the given texts prefilled', () => {
    expect(initialOverride(FOLLOWS_ORG, ORG)).toEqual({
      current: FOLLOWS_ORG,
      choice: 'ORGANISATION',
      buy: ORG.buyFormula,
      sell: ORG.sellFormula,
    });
    expect(initialOverride(FOLLOWS_DEFAULT, ORG)).toMatchObject({ choice: 'ORGANISATION', buy: ORG.buyFormula });
  });
});
