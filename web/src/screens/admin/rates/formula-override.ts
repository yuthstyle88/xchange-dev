// Pricing formula FE T8 (docs/design/blueprint/pricing-formula-web.md §2, §3 O1–O8): the branch override's state and
// what `ratePolicySave` is sent for it (rate_policy_save.rs:74-82: absent = keep, null = follow, text = own).
// SKELETON: signatures fixed; the bodies are ticket T8.
import type { BranchFormulaDto, FormulaTextsDto } from '../../../shared/api/generated/pricing_formula';
import type { RatePolicySaveInput } from '../../../shared/api/generated/rate_policy_save';

export type OverrideChoice = 'ORGANISATION' | 'BRANCH';

export interface FormulaOverrideState {
  /** This branch's entry of the `pricing_formula` view; `null` = never loaded (block closed, or the read failed). */
  current: BranchFormulaDto | null;
  choice: OverrideChoice;
  buy: string;
  sell: string;
}

export const EMPTY_OVERRIDE: FormulaOverrideState = {
  current: null,
  choice: 'ORGANISATION',
  buy: '',
  sell: '',
};

export function initialOverride(entry: BranchFormulaDto, org: FormulaTextsDto): FormulaOverrideState {
  void entry;
  void org;
  throw new Error('todo: pricing-formula-web T8 initialOverride');
}

export function overridePayload(
  s: FormulaOverrideState,
): Pick<RatePolicySaveInput, 'buyFormula' | 'sellFormula'> {
  void s;
  throw new Error('todo: pricing-formula-web T8 overridePayload');
}

export function overrideBlank(s: FormulaOverrideState): 'buyFormula' | 'sellFormula' | null {
  void s;
  throw new Error('todo: pricing-formula-web T8 overrideBlank');
}
