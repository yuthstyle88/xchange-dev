// RD-3 board rows (design rate-display.md §3, D6).
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import { formatRate } from '../../../shared/ui/rate';

export interface BoardRow {
  currency: string;
  /** `null` for the `ALL` group; the group's name otherwise (Q-2 A). */
  group: string | null;
  /** `unitSize` + code, e.g. `100 JPY`. */
  unitLabel: string;
  /** `formatRate(rate, '1', code)`, or `—` for a null side. */
  buy: string;
  sell: string;
}

export function boardRows(data: PublishedRatesData): BoardRow[] {
  return (data.lines ?? []).map((l) => ({
    currency: l.currency,
    group: l.group === 'ALL' ? null : l.group,
    unitLabel: `${l.unitSize} ${l.currency}`,
    buy: l.buyRate !== null ? formatRate(l.buyRate, '1', l.currency) : '—',
    sell: l.sellRate !== null ? formatRate(l.sellRate, '1', l.currency) : '—',
  }));
}
