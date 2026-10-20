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
