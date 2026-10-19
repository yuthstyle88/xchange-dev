// PROVISIONAL until the BE view lands; replaced by shared/api/generated/pricing_board.ts (and the generated command
// types). Shapes per docs/design/blueprint/pricing-tiers.md §6 (view `pricing_board`, `pricingConfigure`,
// `pricingApprove`) and §9; decimals are strings end to end (rate-display.md B7, W7: the page computes no figure).
// Every component of this screen imports its types from this file only, so the swap is one import line per file.

export type MarketSourceCode = 'TT' | 'SR' | 'BOT';

/** One market source as the board shows it (§6: name, fetched_at, stale, error). */
export interface PricingSource {
  source: MarketSourceCode;
  name: string;
  fetchedAt: string | null;
  stale: boolean;
  /** The last fetch's failure, worded by the server; null when it worked. */
  error: string | null;
}

/** One source's quote for one currency (§5.1 `MarketQuote`). */
export interface MarketQuoteView {
  source: MarketSourceCode;
  buy: string | null;
  sell: string | null;
  unitSize: string;
  fetchedAt: string | null;
  stale: boolean;
}

export type TierNo = 1 | 2 | 3 | 4;

/** The branch's THB band of one tier (§3: bands stay per branch). */
export interface TierBandView {
  tierNo: TierNo;
  lowerThb: string;
  /** null on tier 4 (no upper bound). */
  upperInclusiveThb: string | null;
}

export interface TierMarginView {
  tierNo: TierNo;
  buyMarginPct: string;
  sellMarginPct: string;
}

export interface TierPriceView {
  tierNo: TierNo;
  buy: string | null;
  sell: string | null;
}

/** §5.1 `PricingFlag`. */
export type PricingFlagCode =
  | 'NoAnchor'
  | 'NoTtQuote'
  | 'StaleSource'
  | 'MinSpread'
  | 'BelowCost'
  | 'LargeMove'
  | 'Precision';

/** V.9: these keep a row out of bulk approve (§5.1 `blocking()`). */
export const BLOCKING_FLAGS: readonly PricingFlagCode[] = ['BelowCost', 'StaleSource', 'Precision'];

/** §5.1 `RowStatus` (V.9's STATUS). */
export type RowStatusCode = 'Current' | 'Pending' | 'Unpriced' | 'Tiers' | 'Review' | 'Manual' | 'Reference';

/** The page filter (§9 `Pricing`). */
export type PricingFilter = 'all' | 'toApprove' | 'review' | 'current';

/** [proposed] by the FE-desk/pricing sub-controller until V.9's STATUS semantics arrive (P-2): which statuses each
 *  filter shows. `all` shows every row. */
export const FILTER_STATUSES: Record<Exclude<PricingFilter, 'all'>, readonly RowStatusCode[]> = {
  toApprove: ['Tiers', 'Pending', 'Unpriced'],
  review: ['Review'],
  current: ['Current'],
};

export interface PricingSuggestionView {
  /** V.9's suggested tier prices (§5.2). */
  tiers: TierPriceView[];
  /** The margins those prices make over today's base, computed by the server (§5.2). */
  margins: TierMarginView[];
}

/** One currency of the board (§6). */
export interface PricingRowView {
  currency: string;
  /** Enabled currencies are listed first; the others fold below (§9 `PricingGrid`). */
  enabled: boolean;
  unitSize: string;
  quotes: MarketQuoteView[];
  /** The current published tier prices (tier 1 = the shop rate). */
  published: TierPriceView[];
  /** The effective margins (the currency's override, else the branch default, §3). */
  margins: TierMarginView[];
  overridden: boolean;
  minSpreadPct: string | null;
  suggestion: PricingSuggestionView | null;
  flags: PricingFlagCode[];
  status: RowStatusCode;
  /** The draft waiting in A06 for this currency, when `status` is `Review`. */
  draftId: string | null;
}

/** §5.1 `Recommendation`. */
export interface RecommendationView {
  severity: 'High' | 'Warn' | 'Medium' | 'Low';
  code: string;
  currency: string | null;
  side: 'BUY' | 'SELL' | null;
}

/** §7/§9 `OrderRefs`: a recent order and the market it was priced against. */
export interface OrderRefView {
  transactionId: string;
  documentRef: string | null;
  recordedAt: string;
  currency: string;
  market: MarketQuoteView[];
}

/** The `pricing_board` view's data (§6). */
export interface PricingBoardData {
  branch: string;
  /** The policy version the page is built on; approve/configure send it back (lost-update check, §6). */
  policyVersionId: string;
  bands: TierBandView[];
  sources: PricingSource[];
  rows: PricingRowView[];
  recommendations: RecommendationView[];
  orderRefs: OrderRefView[];
}

export const VIEW_PRICING_BOARD = 'pricing_board';

/** `pricingApprove` (§6). The page sends tier PRICES; the server stores the margin each produces (§2, §5.2). */
export interface PricingApproveInput {
  branch: string;
  requestId: string;
  policyVersionId: string;
  currencies: { currency: string; tiers: TierPriceView[] }[];
}
export interface PricingApproveOutput {
  draftId: string;
  /** In bound ⇒ published; out of bound ⇒ waits in A06 (§6). */
  state: 'PUBLISHED' | 'PENDING_APPROVAL';
}
export const ROUTE_PRICING_APPROVE = '/api/pricing/approve';

/** `pricingConfigure` (§6): one currency's override margins and min spread; `margins: null` clears the override. */
export interface PricingConfigureInput {
  branch: string;
  requestId: string;
  policyVersionId: string;
  currency: string;
  margins: TierMarginView[] | null;
  minSpreadPct: string | null;
}
export interface PricingConfigureOutput {
  policyVersionId: string;
  version: number;
}
export const ROUTE_PRICING_CONFIGURE = '/api/pricing/configure';

/** `rateReferenceFetch` with `provider` (§6); allowed for non-BOT sources only with `MarketRatesRefresh` (§8). */
export interface RateReferenceFetchInput {
  branch: string;
  provider: MarketSourceCode;
}
export const ROUTE_RATE_REFERENCE_FETCH = '/api/rate-references/fetch';
