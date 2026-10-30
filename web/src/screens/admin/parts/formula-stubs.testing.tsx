// Pricing formula FE (docs/design/blueprint/pricing-formula-web.md §4): stand-ins for the shared formula parts, used by
// the up-front tests of T6, T7 and T9 through `vi.mock`, so those tickets do not wait for T1–T5. Test-only: no
// production file imports this. Written by the sub-controller claude/studio-claude-3. Do not edit.
import { createSignal, For, Show, type JSX } from 'solid-js';
import { ApiError } from '../../../shared/api/client';
import type { PricingFormulaPreviewOutput } from '../../../shared/api/generated/pricing_formula_preview';
import type { FormulaEditorProps } from './FormulaEditor';
import type { FormulaPreview, FormulaPreviewOptions } from './formula-preview';
import type { FormulaPreviewTableProps } from './FormulaPreviewTable';
import type { FormulaRefusalProps } from './FormulaRefusal';
import type { SpreadFailureLine } from './spread-failure';

// The same values as `formula-preview.ts` (re-exporting from it would loop: that module is mocked to this one).
export const PREVIEW_DELAY_MS = 400;
export const ROUTE_PRICING_FORMULA_PREVIEW = '/api/pricing/formula/preview';
export const ROUTE_PRICING_FORMULA_SAVE = '/api/pricing/formula';

/** Every `createFormulaPreview` call's options, in order. */
export const previewCalls: FormulaPreviewOptions[] = [];
const [stubResult, setStubResult] = createSignal<PricingFormulaPreviewOutput | null>(null);
export { setStubResult };

export function resetFormulaStubs(): void {
  previewCalls.length = 0;
  setStubResult(null);
}

export function createFormulaPreview(o: FormulaPreviewOptions): FormulaPreview {
  previewCalls.push(o);
  return { result: stubResult, pending: () => false, error: () => null, stopped: () => false };
}

export function FormulaEditor(props: FormulaEditorProps): JSX.Element {
  return (
    <div data-stub="formula-editor">
      <textarea
        data-field="buyFormula"
        value={props.buy()}
        disabled={props.disabled()}
        onInput={(e) => props.onBuy(e.currentTarget.value)}
      />
      <textarea
        data-field="sellFormula"
        value={props.sell()}
        disabled={props.disabled()}
        onInput={(e) => props.onSell(e.currentTarget.value)}
      />
      <Show when={props.parseError()}>
        {(pe) => <span data-stub="parse-error">{`${pe().field}:${pe().code}`}</span>}
      </Show>
    </div>
  );
}

export function FormulaPreviewTable(props: FormulaPreviewTableProps): JSX.Element {
  return <div data-stub="preview-table">{String(props.result()?.rows.length ?? 'none')}</div>;
}

export function FormulaRefusal(props: FormulaRefusalProps): JSX.Element {
  return (
    <Show when={props.lines().length > 0}>
      <ul data-stub="refusal">
        <For each={props.lines()}>{(l) => <li>{l.raw}</li>}</For>
      </ul>
    </Show>
  );
}

const CODES = 'syntax|unknownInput|missingInput|divideByZero|overflow|notPositive|tooLong|tooDeep';
const LINE = new RegExp(`^(\\S+) (\\S+)(?: tier (\\d))?( FOLLOW)? (?:(?:buy|sell) \\S+ (?:> sell \\S+|outside guard .+)|(?:${CODES}))$`);

/** A small stand-in for T2's parser: enough for the screen tests, not the contract (that is T2's tests). */
export function parseSpreadFailure(line: string): SpreadFailureLine | null {
  const m = LINE.exec(line);
  if (!m) return null;
  return {
    branch: m[1],
    currency: m[2],
    tier: m[3] ? Number(m[3]) : null,
    follow: Boolean(m[4]),
    kind: 'buyAboveSell',
    buy: null,
    sell: null,
    side: null,
    guardMin: null,
    guardMax: null,
    pct: null,
    code: null,
    raw: line,
  };
}

export function spreadRefusal(err: unknown): SpreadFailureLine[] | null {
  if (!(err instanceof ApiError) || err.status !== 400 || err.code !== 'validationFailed') return null;
  const lines = err.message.split('\n').map(parseSpreadFailure);
  return lines.every((l) => l !== null) ? (lines as SpreadFailureLine[]) : null;
}

export function failureText(line: SpreadFailureLine): string {
  return line.raw;
}
