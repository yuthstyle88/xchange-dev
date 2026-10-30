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
  if (entry.source === 'BRANCH' && entry.own) {
    return { current: entry, choice: 'BRANCH', buy: entry.own.buyFormula, sell: entry.own.sellFormula };
  }
  return { current: entry, choice: 'ORGANISATION', buy: org.buyFormula, sell: org.sellFormula };
}

export function overridePayload(
  s: FormulaOverrideState,
): Pick<RatePolicySaveInput, 'buyFormula' | 'sellFormula'> {
  if (!s.current) return {};
  if (s.choice === 'ORGANISATION') {
    return s.current.source === 'BRANCH' ? { buyFormula: null, sellFormula: null } : {};
  }
  const buyFormula = s.buy.trim();
  const sellFormula = s.sell.trim();
  if (
    s.current.source === 'BRANCH' &&
    s.current.own &&
    buyFormula === s.current.own.buyFormula.trim() &&
    sellFormula === s.current.own.sellFormula.trim()
  ) {
    return {};
  }
  return { buyFormula, sellFormula };
}

export function overrideBlank(s: FormulaOverrideState): 'buyFormula' | 'sellFormula' | null {
  if (s.choice !== 'BRANCH') return null;
  if (!s.buy.trim()) return 'buyFormula';
  if (!s.sell.trim()) return 'sellFormula';
  return null;
}
