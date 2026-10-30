// Pricing formula FE T1 (docs/design/blueprint/pricing-formula-web.md §2, §3 P1–P8): the live preview. Calls
// `pricingFormulaPreview` once the texts stop changing for `PREVIEW_DELAY_MS`, and keeps only the newest answer.
// SKELETON: signatures fixed; the body is ticket T1.
import { createEffect, createSignal, onCleanup } from 'solid-js';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import type { PricingFormulaPreviewOutput } from '../../../shared/api/generated/pricing_formula_preview';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { wordedError } from '../../../shared/i18n/worded-error';

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
  const [result, setResult] = createSignal<PricingFormulaPreviewOutput | null>(null);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [stopped, setStopped] = createSignal(false);
  const write = keyedWrite<
    { branch: string; buyFormula: string; sellFormula: string; scope: FormulaScope },
    PricingFormulaPreviewOutput
  >(o.api, 'POST', ROUTE_PRICING_FORMULA_PREVIEW);
  let seq = 0;
  let lastBranch = o.branch();

  createEffect(() => {
    const branch = o.branch();
    const buyFormula = o.buy().trim();
    const sellFormula = o.sell().trim();
    const enabled = o.enabled();

    if (branch !== lastBranch) {
      lastBranch = branch;
      setStopped(false);
    }

    const mySeq = ++seq;
    setPending(false);

    if (stopped()) return;

    if (!enabled || !buyFormula || !sellFormula) {
      setResult(null);
      setError(null);
      return;
    }

    const timer = setTimeout(() => {
      setPending(true);
      setError(null);
      write
        .send({ branch, buyFormula, sellFormula, scope: o.scope })
        .then((res) => {
          if (mySeq !== seq) return;
          setResult(res);
          setPending(false);
        })
        .catch((err: unknown) => {
          if (mySeq !== seq) return;
          setResult(null);
          setError(wordedError(err));
          setPending(false);
          if (err instanceof ApiError && err.status === 403) {
            setStopped(true);
          }
        });
    }, PREVIEW_DELAY_MS);

    onCleanup(() => clearTimeout(timer));
  });

  return { result, pending, error, stopped };
}
