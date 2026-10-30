// Pricing formula FE T4 (docs/design/blueprint/pricing-formula-web.md §2, §3 T1r–T5r): the live preview's table,
// the per-unit note and the `notChecked` list. SKELETON: the body is ticket T4.
import type { JSX } from 'solid-js';
import type { PricingFormulaPreviewOutput } from '../../../shared/api/generated/pricing_formula_preview';
import './formula.css';

export interface FormulaPreviewTableProps {
  result: () => PricingFormulaPreviewOutput | null;
  pending: () => boolean;
  error: () => string | null;
}

export function FormulaPreviewTable(props: FormulaPreviewTableProps): JSX.Element {
  void props;
  return null;
}
