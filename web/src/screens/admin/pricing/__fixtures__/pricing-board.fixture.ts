// A mocked `pricing_board` answer covering every row status, every flag and every source state of
// docs/design/blueprint/pricing-tiers.md §11, plus a fetch that serves it. Test data only (invented figures).
import { makeMe, makeMeRole, type Me } from '../../../../shared/testing/fixtures/common';
import {
  makePricingBandDto,
  makePricingBoardData,
  makePricingQuoteDto,
  makePricingRecommendationDto,
  makePricingRowDto,
  makePricingSourceDto,
  makePricingTierMarginDto,
  makePricingTierRateDto,
  type PricingBoardData,
  type PricingQuoteDto,
  type PricingRowDto,
  type PricingTierMarginDto,
  type PricingTierRateDto,
} from '../../../../shared/testing/fixtures/pricing_board';
import { BLOCKING_FLAGS, type PricingFlagCode, type RowStatusCode } from '../pricing-labels';

const tiers = (buy: string[], sell: string[]): PricingTierRateDto[] =>
  ([1, 2, 3, 4] as const).map((n, i) =>
    makePricingTierRateDto({ tierNo: n, buy: buy[i] ?? null, sell: sell[i] ?? null }),
  );
const margins = (buy: string[], sell: string[]): PricingTierMarginDto[] =>
  ([1, 2, 3, 4] as const).map((n, i) =>
    makePricingTierMarginDto({ tierNo: n, buyMarginPct: buy[i]!, sellMarginPct: sell[i]! }),
  );
const quote = (
  source: 'TT' | 'SR' | 'BOT',
  buy: string | null,
  sell: string | null,
  stale = false,
): PricingQuoteDto =>
  makePricingQuoteDto({
    source,
    buy,
    sell,
    fetchedAt: '2026-10-19T08:00:00Z',
    stale,
  });

function row(
  currency: string,
  status: RowStatusCode,
  flags: PricingFlagCode[],
  over: Partial<PricingRowDto> = {},
): PricingRowDto {
  const suggestion =
    over.suggestion !== undefined
      ? over.suggestion
      : tiers(['35.02', '35.07', '35.12', '35.17'], ['35.48', '35.43', '35.38', '35.33']);
  const selectable = suggestion !== null && !flags.some((f) => BLOCKING_FLAGS.includes(f));
  return makePricingRowDto({
    currency,
    enabled: true,
    unitSize: '1',
    anchorSource: 'MARKET_TT',
    quotes: [
      quote('TT', '35.10', '35.40'),
      quote('SR', '35.05', '35.45'),
      quote('BOT', '35.00', '35.50'),
    ],
    published: tiers(['35.00', '35.05', '35.10', '35.15'], ['35.50', '35.45', '35.40', '35.35']),
    margins: margins(
      ['0.0000', '0.1000', '0.2000', '0.3000'],
      ['0.0000', '-0.1000', '-0.2000', '-0.3000'],
    ),
    overridden: false,
    minSpreadPct: null,
    suggestion,
    flags,
    selectable,
    status,
    ...over,
  });
}

/** One row per status; flags spread so every flag appears; one disabled currency. */
export const BOARD: PricingBoardData = makePricingBoardData({
  branch: 'MAIN',
  policyVersionId: 'pv-7',
  valueTiersOn: true,
  anchorSource: 'MARKET_TT',
  bands: [
    makePricingBandDto({ tierNo: 1, lowerExclusiveThb: null, upperInclusiveThb: '10000.00' }),
    makePricingBandDto({ tierNo: 2, lowerExclusiveThb: '10000.00', upperInclusiveThb: '50000.00' }),
    makePricingBandDto({ tierNo: 3, lowerExclusiveThb: '50000.00', upperInclusiveThb: '200000.00' }),
    makePricingBandDto({ tierNo: 4, lowerExclusiveThb: '200000.00', upperInclusiveThb: null }),
  ],
  sources: [
    makePricingSourceDto({ source: 'TT', fetchedAt: '2026-10-19T08:00:00Z', stale: false, error: null }),
    makePricingSourceDto({ source: 'SR', fetchedAt: '2026-10-18T20:00:00Z', stale: true, error: null }),
    makePricingSourceDto({ source: 'BOT', fetchedAt: null, stale: false, error: 'source unreachable' }),
  ],
  rows: [
    row('USD', 'Tiers', ['LargeMove']),
    row('EUR', 'Tiers', ['BelowCost']),
    row('JPY', 'Pending', ['StaleSource']),
    row('GBP', 'Unpriced', ['Precision'], { published: tiers([], []) }),
    row('CNY', 'Current', ['MinSpread'], { overridden: true, minSpreadPct: '0.5000' }),
    row('SGD', 'Review', []),
    row('KRW', 'Manual', ['NoAnchor'], { suggestion: null, quotes: [quote('BOT', '0.0260', '0.0270')] }),
    row('AUD', 'Tiers', ['NoTtQuote'], { quotes: [quote('SR', '23.10', '23.40'), quote('BOT', '23.00', '23.50')] }),
    row('MYR', 'Reference', [], { enabled: false }),
  ],
  recommendations: [
    makePricingRecommendationDto({ severity: 'High', code: 'BELOW_COST', currency: 'EUR', side: 'BUY' }),
    makePricingRecommendationDto({ severity: 'Warn', code: 'STALE_SOURCE', currency: 'JPY', side: null }),
    makePricingRecommendationDto({ severity: 'Medium', code: 'LARGE_MOVE', currency: 'USD', side: 'SELL' }),
    makePricingRecommendationDto({ severity: 'Low', code: 'MIN_SPREAD', currency: 'CNY', side: null }),
  ],
});

export const DEFAULT_ME: Me = makeMe({
  branch: 'MAIN',
  roles: [makeMeRole({ role: 'EXECUTIVE', branchId: null })],
  permissions: ['ReferenceRatesRead', 'MarketRatesRefresh'],
});

export type Call = { method: string; path: string; key: string | null; body: unknown };

/** A fetch that answers the board view and `me` (an envelope like every v3 view) and records every call. `boards` are answered
 *  in turn (the last one repeats); commands answer `command(path, body)`, default 200 `{}`. */
export function boardFetch(
  boards: PricingBoardData[] = [BOARD],
  command: (path: string, body: unknown) => { status: number; body: unknown } = () => ({ status: 200, body: {} }),
  me: Me = DEFAULT_ME,
) {
  const calls: Call[] = [];
  let n = 0;
  const fn = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0];
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, path, key: new Headers(init?.headers).get('idempotency-key'), body });
    const a =
      method === 'GET' && path === '/api/v3/views/pricing_board'
        ? { status: 200, body: { version: ++n, changed: true, data: boards[Math.min(n - 1, boards.length - 1)] } }
        : method === 'GET' && path === '/api/v3/views/me'
          ? { status: 200, body: { version: 1, changed: true, data: me } }
          : method === 'POST'
            ? command(path, body)
            : { status: 404, body: { error: { code: 'notFound', message: 'nf' } } };
    return new Response(JSON.stringify(a.body), { status: a.status, headers: { 'content-type': 'application/json' } });
  };
  return { fn, calls };
}
