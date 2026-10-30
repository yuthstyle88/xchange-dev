// Pricing formula FE T1 (docs/design/blueprint/pricing-formula-web.md §2, §3 P1–P8): the live preview. Calls
// `pricingFormulaPreview` once the texts stop changing for `PREVIEW_DELAY_MS`, and keeps only the newest answer.
// SKELETON: signatures fixed; the body is ticket T1.
import type { ApiClient } from '../../../shared/api/client';
import type { PricingFormulaPreviewOutput } from '../../../shared/api/generated/pricing_formula_preview';

export type FormulaScope = 'ORGANISATION' | 'BRANCH';

export const PREVIEW_DELAY_MS = 400;
export const ROUTE_PRICING_FORMULA_PREVIEW = '/api/pricing/formula/preview';
export const ROUTE_PRICING_FORMULA_SAVE = '/api/pricing/formula';

export interface FormulaPreviewOptions {
  api: ApiClient;
  branch: () => string;
  scope: FormulaScope;
  buy: () => string;
  sell: () => string;
  enabled: () => boolean;
}

export interface FormulaPreview {
  result: () => PricingFormulaPreviewOutput | null;
  pending: () => boolean;
  /** The worded error of the last failed call, else `null`. */
  error: () => string | null;
  /** True after a 403: no further call until the branch changes. */
  stopped: () => boolean;
}

/** Call inside a component (it owns effects and timers). */
export function createFormulaPreview(o: FormulaPreviewOptions): FormulaPreview {
  void o;
  return {
    result: () => null,
    pending: () => false,
    error: () => null,
    stopped: () => false,
  };
}
