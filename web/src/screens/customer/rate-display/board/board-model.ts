// P01 v2 board model (rate-display.md §9.3, §9.4): the route's answer → headings and rows of raw rate strings.
// Formatting is not done here (decimal-format.ts); this module only arranges the route's strings.
// SKELETON STUB (feat/fe-rateboard): RB-1 replaces `boardModel` and `bandLabel`; the types and signatures stay.
import type { PublishedLineDto, PublishedRatesData } from '../../../../shared/api/generated/published_rates';

/** `PublishedBandHeadingDto` as RB-BE states it (§9.3). Local until the generated type has it. */
export interface BandHeadingIn {
  tier: string;
  lowerExclusiveThb: string | null;
  upperInclusiveThb: string | null;
}

/** `PublishedBandDto` as RB-BE states it (§9.3). Local until the generated type has it. */
export interface BandIn {
  tier: string;
  buyRate: string | null;
  sellRate: string | null;
}

/** The route's answer with RB-BE's two additive fields, optional until the generated type has them. */
export type BoardRatesData = Omit<PublishedRatesData, 'lines'> & {
  bandHeadings?: BandHeadingIn[];
  lines: Array<PublishedLineDto & { bands?: BandIn[] }>;
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

/** One heading's label from its bound strings (§9.4). */
export function bandLabel(_h: BandHeadingIn): string {
  return '';
}

export function boardModel(data: BoardRatesData): BoardModel {
  return {
    headings: [],
    singleColumn: true,
    rows: data.lines.map((l) => ({
      key: `${l.currency}|${l.group}`,
      code: l.currency,
      group: null,
      unitLabel: '',
      cells: [],
    })),
  };
}
