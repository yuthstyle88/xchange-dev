// Pricing formula FE T5 (docs/design/blueprint/pricing-formula-web.md §2, §3 R1–R2): a refused save's failing
// prices, one row per line. SKELETON: the body is ticket T5.
import type { JSX } from 'solid-js';
import type { SpreadFailureLine } from './spread-failure';
import './formula.css';

export interface FormulaRefusalProps {
  lines: () => readonly SpreadFailureLine[];
}

export function FormulaRefusal(props: FormulaRefusalProps): JSX.Element {
  void props;
  return null;
}
