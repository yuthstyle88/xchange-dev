// P01 v2 board model (rate-display.md §9.3, §9.4): the route's answer → headings and rows of raw rate strings.
// Formatting is not done here (decimal-format.ts); this module only arranges the route's strings.
import type {
  PublishedBandDto,
  PublishedBandHeadingDto,
  PublishedLineDto,
  PublishedRatesData,
} from '../../../../shared/api/generated/published_rates';

/** One band heading of the route (RB-BE #3582, §9.3). */
export type BandHeadingIn = PublishedBandHeadingDto;

/** One band of a line (RB-BE #3582, §9.3). */
export type BandIn = PublishedBandDto;

/**
 * The route's answer. `bandHeadings` and `line.bands` stay optional here on purpose: a server older than RB-BE
 * (e.g. during a deploy) sends neither, and the board then reads as tiers off (§9.4) instead of failing.
 */
export type BoardRatesData = Omit<PublishedRatesData, 'bandHeadings' | 'lines'> & {
  bandHeadings?: BandHeadingIn[];
  lines: Array<Omit<PublishedLineDto, 'bands'> & { bands?: BandIn[] }>;
};

export interface BandHeading {
  tier: string;
  /** e.g. `≤ 10,000 ฿`, `10,001–50,000 ฿`, `> 200,000 ฿` (§9.4). */
  label: string;
}

export interface BandCell {
  tier: string;
  /** The route's decimal strings, unformatted. */
  buy: string | null;
  sell: string | null;
  /** True for bands 2..n of a line with no bands on a tiered board: the same rate, shown dimmed. */
  repeat: boolean;
}

export interface BoardRow {
  /** `currency` + `group`, unique on the board; the key for keeping a row's DOM across polls. */
  key: string;
  code: string;
  /** null for `ALL`. */
  group: string | null;
  /** `100 JPY` when the unit is not 1, else empty. */
  unitLabel: string;
  cells: BandCell[];
}

export interface BoardModel {
  headings: BandHeading[];
  /** True when the board has no bands (tiers off, `bandHeadings: []`): one "all amounts" column. */
  singleColumn: boolean;
  rows: BoardRow[];
}

function addCommas(intStr: string): string {
  return intStr.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

function isWholeBaht(s: string): boolean {
  const [, rawFrac = ''] = s.trim().split('.');
  return rawFrac === '' || /^0+$/.test(rawFrac);
}

function formatBound(s: string): string {
  const [rawInt = '0', rawFrac = ''] = s.trim().split('.');
  if (rawFrac === '' || /^0+$/.test(rawFrac)) {
    return addCommas(rawInt);
  }
  return `${addCommas(rawInt)}.${rawFrac}`;
}

function addOneInteger(intStr: string): string {
  const digits = intStr.split('');
  let i = digits.length - 1;
  while (i >= 0) {
    const code = digits[i]!.charCodeAt(0);
    if (code < 57) {
      digits[i] = String.fromCharCode(code + 1);
      return digits.join('');
    }
    digits[i] = '0';
    i--;
  }
  return '1' + digits.join('');
}

function formatUnitLabel(unitSize: string, currency: string): string {
  const [intPart = '1', fracPart = ''] = unitSize.trim().split('.');
  const strippedFrac = fracPart.replace(/0+$/, '');
  const cleanUnit = strippedFrac ? `${intPart}.${strippedFrac}` : intPart;
  if (!cleanUnit || cleanUnit === '1') return '';
  return `${cleanUnit} ${currency}`;
}

/** One heading's label from its bound strings (§9.4). */
export function bandLabel(h: BandHeadingIn): string {
  const lower = h.lowerExclusiveThb;
  const upper = h.upperInclusiveThb;
  if (lower === null && upper !== null) {
    return `≤ ${formatBound(upper)} ฿`;
  }
  if (lower !== null && upper === null) {
    return `> ${formatBound(lower)} ฿`;
  }
  if (lower !== null && upper !== null) {
    if (isWholeBaht(lower) && isWholeBaht(upper)) {
      const [lowerInt = '0'] = lower.trim().split('.');
      return `${addCommas(addOneInteger(lowerInt))}–${formatBound(upper)} ฿`;
    }
    return `> ${formatBound(lower)} – ${formatBound(upper)} ฿`;
  }
  return '';
}

export function boardModel(data: BoardRatesData): BoardModel {
  const rawHeadings = data.bandHeadings ?? [];
  const singleColumn = rawHeadings.length === 0;
  const headings: BandHeading[] = rawHeadings.map((h) => ({
    tier: h.tier,
    label: bandLabel(h),
  }));

  return {
    headings,
    singleColumn,
    rows: data.lines.map((l) => {
      let cells: BandCell[];
      if (singleColumn) {
        cells = [{ tier: 'ALL', buy: l.buyRate, sell: l.sellRate, repeat: false }];
      } else {
        const lineBands = l.bands ?? [];
        if (lineBands.length === 0) {
          cells = headings.map((h, idx) => ({
            tier: h.tier,
            buy: l.buyRate,
            sell: l.sellRate,
            repeat: idx > 0,
          }));
        } else {
          cells = headings.map((h) => {
            const match = lineBands.find((b) => b.tier === h.tier);
            return match
              ? { tier: h.tier, buy: match.buyRate, sell: match.sellRate, repeat: false }
              : { tier: h.tier, buy: null, sell: null, repeat: false };
          });
        }
      }

      return {
        key: `${l.currency}|${l.group}`,
        code: l.currency,
        group: l.group === 'ALL' ? null : l.group,
        unitLabel: formatUnitLabel(l.unitSize, l.currency),
        cells,
      };
    }),
  };
}
