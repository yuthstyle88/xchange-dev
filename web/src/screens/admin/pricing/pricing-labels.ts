// §9 status / flag / source labels and shared pricing constants (#3766, #3987).
import { lang } from '../../../shared/i18n';
import { en, th } from './i18n';

export type MarketSourceCode = 'TT' | 'SR' | 'BOT';
export type TierNo = 1 | 2 | 3 | 4;

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

export type RowStatusCode =
  | 'Current'
  | 'Pending'
  | 'Unpriced'
  | 'Tiers'
  | 'Review'
  | 'Manual'
  | 'Reference';

export type PricingFilter = 'all' | 'toApprove' | 'review' | 'current';

export const FILTER_STATUSES: Record<Exclude<PricingFilter, 'all'>, readonly RowStatusCode[]> = {
  toApprove: ['Tiers', 'Pending', 'Unpriced'],
  review: ['Review'],
  current: ['Current'],
};

export const VIEW_PRICING_BOARD = 'pricing_board';
export const ROUTE_PRICING_APPROVE = '/api/pricing/approve';
export const ROUTE_PRICING_CONFIGURE = '/api/pricing/configure';
export const ROUTE_RATE_REFERENCE_FETCH = '/api/rate-references/fetch';
/** V.9 (pricing-tiers.md §15.8a): PV-f4, PV-d2, PV-j. */
export const ROUTE_PRICING_MODE = '/api/pricing/mode';
export const ROUTE_PRICING_BRANCH_POLICY = '/api/pricing/branch-policy';
export const ROUTE_PRICING_DISPLAY = '/api/pricing/display';
/** PV-g: fetches TT and SR (rateReferenceFetch stays BoT-only); MarketRatesRefresh permission. */
export const ROUTE_PRICING_MARKET_REFRESH = '/api/pricing/market-refresh';
/** The Owner's pricing rules (owner-set-values.md §2, #6555): the view and its EXECUTIVE-only save. */
export const VIEW_PRICING_RULES = 'pricing_rules';
export const ROUTE_PRICING_RULES = '/api/pricing/rules';

/** pricingBranchPolicySave refuses basis points outside this range (400 validationFailed). */
export const BRANCH_BP_MIN = -300;
export const BRANCH_BP_MAX = 300;
/** rateDisplaySave keeps at most this many featured currencies. */
export const DISPLAY_FEATURED_MAX = 10;

export function statusLabel(status: string): string {
  const t = lang() === 'en' ? en : th;
  return t[`status.${status}` as keyof typeof th] ?? '';
}

export function flagLabel(flag: string): string {
  const t = lang() === 'en' ? en : th;
  return t[`flag.${flag}` as keyof typeof th] ?? '';
}

export function sourceLabel(source: string): string {
  const t = lang() === 'en' ? en : th;
  return t[`source.${source}` as keyof typeof th] ?? source;
}

function prefixed(prefix: string, code: string): string {
  const t = lang() === 'en' ? en : th;
  return t[`${prefix}.${code}` as keyof typeof th] ?? code;
}

/** `PricingRowDto.mode`: AUTO · MANUAL · FOLLOW. */
export const modeLabel = (mode: string): string => prefixed('mode', mode);
/** `PricingTierSetDto.source`: MANUAL · MAIN_PUSH · FOLLOW. */
export const tierSetSourceLabel = (source: string): string => prefixed('tierSetSource', source);
/** `PricingFollowResultDto.status`: CANDIDATE · BLOCKED · PUBLISHED. */
export const followStatusLabel = (status: string): string => prefixed('followStatus', status);
/** `PricingFollowResultDto.reasons[]`: NO_ANCHOR · STALE_SOURCE · PRECISION · BELOW_COST · LARGE_MOVE · INVALID_TIERS. */
export const followReasonLabel = (reason: string): string => prefixed('followReason', reason);
/** `PricingApproveCurrencyResult.state`: PUBLISHED · REFUSED. */
export const approveStateLabel = (state: string): string => prefixed('approveState', state);
/** `PricingApproveBranchResult.state`: PUBLISHED · SKIPPED_OWN · FAILED · ALREADY_DONE. */
export const branchPushStateLabel = (state: string): string => prefixed('branchPush', state);
/** `PricingBranchPolicyDto.from`: currency · `*` · default. */
export const policyFromLabel = (from: string): string => prefixed('policyFrom', from);
/** `PricingDisplayDto.from`: branch · MAIN · default. */
export const displayFromLabel = (from: string): string => prefixed('displayFrom', from);
