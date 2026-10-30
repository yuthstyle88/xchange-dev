// Pricing formula FE T3 (docs/design/blueprint/pricing-formula-web.md §2, §3 E1–E4): the two formula texts, the
// inputs a formula may name, and the parse error under its field. SKELETON: the body is ticket T3.
import type { JSX } from 'solid-js';
import type { FormulaTextsDto } from '../../../shared/api/generated/pricing_formula';
import type { FormulaParseErrorDto } from '../../../shared/api/generated/pricing_formula_preview';
import './formula.css';

export interface FormulaEditorProps {
  buy: () => string;
  sell: () => string;
  onBuy: (v: string) => void;
  onSell: (v: string) => void;
  /** `PricingFormulaData.inputs` names, in order. */
  inputs: () => readonly string[];
  defaults: () => FormulaTextsDto;
  parseError: () => FormulaParseErrorDto | null;
  disabled: () => boolean;
}

export function FormulaEditor(props: FormulaEditorProps): JSX.Element {
  void props;
  return null;
}
