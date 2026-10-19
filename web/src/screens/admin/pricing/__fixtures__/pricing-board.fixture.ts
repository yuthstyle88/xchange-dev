// A mocked `pricing_board` answer covering every row status, every flag and every source state of
// docs/design/blueprint/pricing-tiers.md §11, plus a fetch that serves it. Test data only (invented figures).
import type {
  MarketQuoteView,
  PricingBoardData,
  PricingFlagCode,
  PricingRowView,
  RowStatusCode,
  TierMarginView,
  TierPriceView,
} from '../pricing-board.types';

const tiers = (buy: string[], sell: string[]): TierPriceView[] =>
  ([1, 2, 3, 4] as const).map((n, i) => ({ tierNo: n, buy: buy[i] ?? null, sell: sell[i] ?? null }));
const margins = (buy: string[], sell: string[]): TierMarginView[] =>
  ([1, 2, 3, 4] as const).map((n, i) => ({ tierNo: n, buyMarginPct: buy[i], sellMarginPct: sell[i] }));
const quote = (source: 'TT' | 'SR' | 'BOT', buy: string | null, sell: string | null, stale = false): MarketQuoteView => ({
  source,
  buy,
  sell,
  unitSize: '1',
  fetchedAt: '2026-10-19T08:00:00Z',
  stale,
});

function row(currency: string, status: RowStatusCode, flags: PricingFlagCode[], over: Partial<PricingRowView> = {}): PricingRowView {
  return {
    currency,
    enabled: true,
    unitSize: '1',
    quotes: [quote('TT', '35.10', '35.40'), quote('SR', '35.05', '35.45'), quote('BOT', '35.00', '35.50')],
    published: tiers(['35.00', '35.05', '35.10', '35.15'], ['35.50', '35.45', '35.40', '35.35']),
    margins: margins(['0.0000', '0.1000', '0.2000', '0.3000'], ['0.0000', '-0.1000', '-0.2000', '-0.3000']),
    overridden: false,
    minSpreadPct: null,
    suggestion: {
      tiers: tiers(['35.02', '35.07', '35.12', '35.17'], ['35.48', '35.43', '35.38', '35.33']),
      margins: margins(['0.0571', '0.2000', '0.3429', '0.4857'], ['-0.0563', '-0.1972', '-0.3380', '-0.4789']),
    },
    flags,
    status,
    draftId: null,
    ...over,
  };
}

/** One row per status; flags spread so every flag appears; one disabled currency. */
export const BOARD: PricingBoardData = {
  branch: 'MAIN',
  policyVersionId: 'pv-7',
  bands: [
    { tierNo: 1, lowerThb: '0.00', upperInclusiveThb: '10000.00' },
    { tierNo: 2, lowerThb: '10000.01', upperInclusiveThb: '50000.00' },
    { tierNo: 3, lowerThb: '50000.01', upperInclusiveThb: '200000.00' },
    { tierNo: 4, lowerThb: '200000.01', upperInclusiveThb: null },
  ],
  sources: [
    { source: 'TT', name: 'TT Pattaya', fetchedAt: '2026-10-19T08:00:00Z', stale: false, error: null },
    { source: 'SR', name: 'Super Rich', fetchedAt: '2026-10-18T20:00:00Z', stale: true, error: null },
    { source: 'BOT', name: 'BoT', fetchedAt: null, stale: false, error: 'source unreachable' },
  ],
  rows: [
    row('USD', 'Tiers', ['LargeMove']),
    row('EUR', 'Tiers', ['BelowCost']),
    row('JPY', 'Pending', ['StaleSource']),
    row('GBP', 'Unpriced', ['Precision'], { published: tiers([], []) }),
    row('CNY', 'Current', ['MinSpread'], { overridden: true, minSpreadPct: '0.5000' }),
    row('SGD', 'Review', [], { draftId: 'draft-42' }),
    row('KRW', 'Manual', ['NoAnchor'], { suggestion: null, quotes: [quote('BOT', '0.0260', '0.0270')] }),
    row('AUD', 'Tiers', ['NoTtQuote'], { quotes: [quote('SR', '23.10', '23.40'), quote('BOT', '23.00', '23.50')] }),
    row('MYR', 'Reference', [], { enabled: false }),
  ],
  recommendations: [
    { severity: 'High', code: 'BELOW_COST', currency: 'EUR', side: 'BUY' },
    { severity: 'Warn', code: 'STALE_SOURCE', currency: 'JPY', side: null },
    { severity: 'Medium', code: 'LARGE_MOVE', currency: 'USD', side: 'SELL' },
    { severity: 'Low', code: 'MIN_SPREAD', currency: 'CNY', side: null },
  ],
  orderRefs: [
    {
      transactionId: 'tx-1',
      documentRef: 'XC-20261019-000001',
      recordedAt: '2026-10-19T09:00:00Z',
      currency: 'USD',
      market: [quote('TT', '35.10', '35.40'), quote('SR', '35.05', '35.45')],
    },
  ],
};

export type Call = { method: string; path: string; key: string | null; body: unknown };

/** A fetch that answers the board view (an envelope like every v3 view) and records every call. `boards` are answered
 *  in turn (the last one repeats); commands answer `command(path, body)`, default 200 `{}`. */
export function boardFetch(
  boards: PricingBoardData[] = [BOARD],
  command: (path: string, body: unknown) => { status: number; body: unknown } = () => ({ status: 200, body: {} }),
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
        : method === 'POST'
          ? command(path, body)
          : { status: 404, body: { error: { code: 'notFound', message: 'nf' } } };
    return new Response(JSON.stringify(a.body), { status: a.status, headers: { 'content-type': 'application/json' } });
  };
  return { fn, calls };
}
