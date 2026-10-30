// Pricing formula FE T2 (docs/design/blueprint/pricing-formula-web.md §2, §3 S1–S10): read the save check's failure
// lines (`SpreadFailure::describe`, api/crates/xc-rate/src/application/formula/spread_check.rs:92-130) into rows.
import { ApiError } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import * as table from './formula-i18n';

const t = screenT('admin.formula', table);

const HEAD = /^(\S+) (\S+)(?: tier (\d+))?( FOLLOW)? (.+)$/;
const BUY_ABOVE_SELL = /^buy (\S+) > sell (\S+)$/;
const OUTSIDE_GUARD = /^(buy|sell) (\S+) outside guard \[(\S+), (\S+)\] \((\S+) %\)$/;
const FORMULA_CODES = new Set([
  'syntax',
  'unknownInput',
  'missingInput',
  'divideByZero',
  'overflow',
  'notPositive',
  'tooLong',
  'tooDeep',
]);

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
  const m = HEAD.exec(line);
  if (!m) return null;
  const base = {
    branch: m[1]!,
    currency: m[2]!,
    tier: m[3] ? Number(m[3]) : null,
    follow: Boolean(m[4]),
    raw: line,
  };
  const detail = m[5]!;

  const bas = BUY_ABOVE_SELL.exec(detail);
  if (bas) {
    return {
      ...base,
      kind: 'buyAboveSell',
      buy: bas[1] === '-' ? null : bas[1]!,
      sell: bas[2] === '-' ? null : bas[2]!,
      side: null,
      guardMin: null,
      guardMax: null,
      pct: null,
      code: null,
    };
  }

  if (FORMULA_CODES.has(detail)) {
    return {
      ...base,
      kind: 'formula',
      buy: null,
      sell: null,
      side: null,
      guardMin: null,
      guardMax: null,
      pct: null,
      code: detail,
    };
  }

  const og = OUTSIDE_GUARD.exec(detail);
  if (og) {
    const side = og[1] as 'buy' | 'sell';
    const price = og[2] === '-' ? null : og[2]!;
    return {
      ...base,
      kind: 'outsideGuard',
      buy: side === 'buy' ? price : null,
      sell: side === 'sell' ? price : null,
      side,
      guardMin: og[3]!,
      guardMax: og[4]!,
      pct: og[5]!,
      code: null,
    };
  }

  return null;
}

/** The failure lines of a save refusal, or `null` when `err` is not one (any other refusal keeps its own path). */
export function spreadRefusal(err: unknown): SpreadFailureLine[] | null {
  if (!(err instanceof ApiError) || err.status !== 400 || err.code !== 'validationFailed' || !err.message) {
    return null;
  }
  const out: SpreadFailureLine[] = [];
  for (const raw of err.message.split('\n')) {
    const parsed = parseSpreadFailure(raw);
    if (!parsed) return null;
    out.push(parsed);
  }
  return out;
}

/** The Thai text of a line's problem, with its numbers (`parts/formula-i18n.ts`). */
export function failureText(line: SpreadFailureLine): string {
  const dash = t('dash');
  if (line.kind === 'buyAboveSell') {
    return t('buyAboveSell', { buy: line.buy ?? dash, sell: line.sell ?? dash });
  }
  if (line.kind === 'outsideGuard') {
    const isSell = line.side === 'sell';
    return t(isSell ? 'outsideGuardSell' : 'outsideGuardBuy', {
      price: (isSell ? line.sell : line.buy) ?? dash,
      min: line.guardMin ?? dash,
      max: line.guardMax ?? dash,
      pct: line.pct ?? dash,
    });
  }
  return t(`code.${line.code}` as table.Key);
}
