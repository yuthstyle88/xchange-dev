// Pricing formula FE T7 (docs/design/blueprint/pricing-formula-web.md §1, §3 V1–V8): the branch override block at the
// foot of the policy card. Collapsed; reads `pricing_formula` only when opened (RatesDraft.test.tsx:607 pins the
// reads on show). SKELETON: the body is ticket T7.
import type { JSX } from 'solid-js';
import type { FormulaOverrideState } from './formula-override';
import '../parts/formula.css';

export interface RatesFormulaOverrideProps {
  branch: () => string;
  state: () => FormulaOverrideState;
  setState: (s: FormulaOverrideState) => void;
  /** Moves after a successful policy save: an open block reads the view again. */
  reloadKey: () => number;
  disabled: () => boolean;
}

export function RatesFormulaOverride(props: RatesFormulaOverrideProps): JSX.Element {
  void props;
  return null;
}
