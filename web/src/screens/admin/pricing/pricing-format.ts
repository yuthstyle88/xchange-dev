// Display helpers for the pricing cards (look only, owner 2026-11-05 "ทำให้หน้าตามันดู มีราคา หน่อย").
// No rate is parsed into a float for display. `pricing_board` carries no per-currency scale (the server sends its
// numeric(20,8) strings, e.g. "23.15000000"), so a currency's decimals are read from its own figures on the board: the
// most fraction digits any of its rates keeps once trailing zeros are trimmed (`rowDigits`). Every rate of that
// currency is shown at that many places. Nothing here is a hard-coded number of decimals.
import type {
  PricingBandDto,
  PricingRecommendationDto,
  PricingRowDto,
} from '../../../shared/api/generated/pricing_board';

const DASH = '—';

/** `"23.15000000"` → `"23.15"`, `"36.00000000"` → `"36"`; anything that is not a plain decimal is returned as sent. */
export function trimRate(value: string): string {
  const s = value.trim();
  if (!/^-?\d+(\.\d+)?$/.test(s)) return s;
  if (!s.includes('.')) return s;
  return s.replace(/0+$/, '').replace(/\.$/, '');
}

function fracLen(trimmed: string): number {
  const i = trimmed.indexOf('.');
  return i < 0 ? 0 : trimmed.length - i - 1;
}

function padTo(trimmed: string, digits: number): string {
  if (digits === 0 || !/^-?\d+(\.\d+)?$/.test(trimmed)) return trimmed;
  const have = fracLen(trimmed);
  if (have >= digits) return trimmed;
  return `${have === 0 ? `${trimmed}.` : trimmed}${'0'.repeat(digits - have)}`;
}

/** Every rate the board carries for one currency: quotes, the publication, the suggestion and the tier set. */
function rowRates(row: PricingRowDto): (string | null)[] {
  const out: (string | null)[] = [];
  for (const q of row.quotes) out.push(q.buy, q.sell);
  for (const p of row.published) out.push(p.buy, p.sell);
  for (const p of row.suggestion ?? []) out.push(p.buy, p.sell);
  for (const p of row.tierSet?.tiers ?? []) out.push(p.buy, p.sell);
  return out;
}

/** The currency's display decimals, read from its own figures (see the file header). */
export function rowDigits(row: PricingRowDto): number {
  return Math.max(
    0,
    ...rowRates(row).map((v) => (v != null && v.trim() !== '' ? fracLen(trimRate(v)) : 0)),
  );
}

/** One rate at a currency's display decimals: trailing zeros trimmed, then padded back to `digits` (never cut). */
export function rateAt(value: string | null | undefined, digits: number): string {
  if (value == null || value.trim() === '') return DASH;
  return padTo(trimRate(value), digits);
}

/**
 * The signed % gap of `value` against `base` for a hint beside the figures ("▲ 0.20%"). Display only: the server
 * keeps every figure that is stored or compared; this number is never sent anywhere.
 */
export function pctGap(value: string | null | undefined, base: string | null | undefined): number | null {
  if (value == null || base == null) return null;
  const v = Number(value);
  const b = Number(base);
  if (!Number.isFinite(v) || !Number.isFinite(b) || b === 0) return null;
  return (v / b - 1) * 100;
}

/** `+2.05%` / `-0.94%` (two places, the way the shop's own board prints a gap). */
export function pctText(gap: number): string {
  const s = Math.abs(gap).toFixed(2);
  return `${gap > 0 ? '+' : gap < 0 ? '-' : ''}${s}%`;
}

/** `"10000.00"` → `"10,000"`: whole-baht amounts lose their zero fraction and gain thousands separators. */
export function thbText(value: string): string {
  const t = trimRate(value);
  const [int, frac] = t.split('.');
  const grouped = (int ?? '').replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return frac ? `${grouped}.${frac}` : grouped;
}

/** The first whole baht above an exclusive lower bound ("10000.00" → "10,001"), the shop board's wording; a bound
 *  with a fraction is shown as it is. */
export function thbAbove(lowerExclusive: string): string {
  const t = trimRate(lowerExclusive);
  if (!/^\d+$/.test(t)) return thbText(t);
  return thbText((BigInt(t) + 1n).toString());
}

export type BandWords = {
  upTo: (upper: string) => string;
  between: (lower: string, upper: string) => string;
  above: (lower: string) => string;
};

/** "ไม่เกิน 10,000 บาท" · "10,001–50,000 บาท" · "มากกว่า 200,000 บาท", from the board's bands. */
export function bandText(band: PricingBandDto | undefined, words: BandWords): string {
  if (!band) return '';
  const lower = band.lowerExclusiveThb;
  const upper = band.upperInclusiveThb;
  if (lower == null && upper != null) return words.upTo(thbText(upper));
  if (lower != null && upper != null) return words.between(thbAbove(lower), thbText(upper));
  if (lower != null) return words.above(thbText(lower));
  return '';
}

const NAME_CACHE = new Map<string, Intl.DisplayNames | null>();

/** The currency's name in the page language (Intl's CLDR names); blank when unknown. */
export function currencyName(code: string, lang: string): string {
  if (!NAME_CACHE.has(lang)) {
    NAME_CACHE.set(
      lang,
      typeof Intl !== 'undefined' && typeof Intl.DisplayNames === 'function'
        ? new Intl.DisplayNames([lang], { type: 'currency' })
        : null,
    );
  }
  const names = NAME_CACHE.get(lang);
  if (!names) return '';
  try {
    const n = names.of(code);
    return n && n !== code ? n : '';
  } catch {
    return '';
  }
}

/** The market source a card leads with: the row's anchor when it has a quote, else TT, SR, BoT in that order. */
export function primarySource(row: PricingRowDto): 'TT' | 'SR' | 'BOT' {
  const has = (s: string) => row.quotes.some((q) => q.source === s && (q.buy != null || q.sell != null));
  const anchor = row.anchorSource === 'MARKET_SR' ? 'SR' : row.anchorSource === 'BOT' ? 'BOT' : 'TT';
  if (has(anchor)) return anchor;
  for (const s of ['TT', 'SR', 'BOT'] as const) if (has(s)) return s;
  return anchor;
}

/** Recommendation code for a currency the market lists and the branch does not sell (pricing-tiers.md §5.2). */
export const NOT_OFFERED = 'NotOffered';

/** Currencies the recommendations flag `NotOffered`, in the order the board sent them. */
export function notOfferedCodes(items: readonly PricingRecommendationDto[]): string[] {
  const out: string[] = [];
  for (const r of items) {
    if (r.code === NOT_OFFERED && r.currency && !out.includes(r.currency)) out.push(r.currency);
  }
  return out;
}

/** A row belongs to the "not opened yet" group when the branch does not sell it, or the recommendations say so. */
export function isUnopened(row: PricingRowDto, notOffered: readonly string[]): boolean {
  return !row.enabled || notOffered.includes(row.currency);
}
