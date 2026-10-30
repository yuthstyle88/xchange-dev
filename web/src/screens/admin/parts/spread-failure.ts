// Pricing formula FE T2 (docs/design/blueprint/pricing-formula-web.md §2, §3 S1–S10): read the save check's failure
// lines (`SpreadFailure::describe`, api/crates/xc-rate/src/application/formula/spread_check.rs:92-130) into rows.
// SKELETON: signatures fixed; the bodies are ticket T2.

export type SpreadFailureKind = 'buyAboveSell' | 'outsideGuard' | 'formula';

export interface SpreadFailureLine {
  branch: string;
  currency: string;
  /** `null` = the single-price line. */
  tier: number | null;
  follow: boolean;
  kind: SpreadFailureKind;
  /** buyAboveSell: both sides (`-` ⇒ null); outsideGuard: the guarded side's price; formula: null. */
  buy: string | null;
  sell: string | null;
  /** outsideGuard only. */
  side: 'buy' | 'sell' | null;
  guardMin: string | null;
  guardMax: string | null;
  /** outsideGuard only, with its sign (`+1.2`). */
  pct: string | null;
  /** formula only: the `FormulaError` code. */
  code: string | null;
  /** The line as received. */
  raw: string;
}

/** One failure line, or `null` when the text is not one. */
export function parseSpreadFailure(line: string): SpreadFailureLine | null {
  void line;
  throw new Error('todo: pricing-formula-web T2 parseSpreadFailure');
}

/** The failure lines of a save refusal, or `null` when `err` is not one (any other refusal keeps its own path). */
export function spreadRefusal(err: unknown): SpreadFailureLine[] | null {
  void err;
  throw new Error('todo: pricing-formula-web T2 spreadRefusal');
}

/** The Thai text of a line's problem, with its numbers (`parts/formula-i18n.ts`). */
export function failureText(line: SpreadFailureLine): string {
  void line;
  throw new Error('todo: pricing-formula-web T2 failureText');
}
