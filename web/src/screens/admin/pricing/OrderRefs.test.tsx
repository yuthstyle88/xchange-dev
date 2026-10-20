// §9 `OrderRefs` (FE pricing-board PB-9, #3765; docs/design/blueprint/pricing-tiers.md §6, §9).
// hidden until the order-reference view exists (design §7)
// Behaviour table (§9):
// - refs: one line per order: document ref (or id), time, currency, each market quote as sent
// - a ref with no market: the `—` text
// - none: nothing rendered
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import { makePricingQuoteDto } from '../../../shared/testing/fixtures/pricing_board';
import { OrderRefs, type OrderRefView } from './OrderRefs';

const SAMPLE_ORDER_REFS: OrderRefView[] = [
  {
    transactionId: 'tx-1',
    documentRef: 'XC-20261019-000001',
    recordedAt: '2026-10-19T09:00:00Z',
    currency: 'USD',
    market: [
      makePricingQuoteDto({ source: 'TT', buy: '35.10', sell: '35.40', fetchedAt: '2026-10-19T08:00:00Z', stale: false }),
      makePricingQuoteDto({ source: 'SR', buy: '35.05', sell: '35.45', fetchedAt: '2026-10-19T08:00:00Z', stale: false }),
    ],
  },
];

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

function mount(refs: readonly OrderRefView[]) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <OrderRefs refs={refs} />, host);
}

describe('OrderRefs', () => {
  it('refs: renders one line per order with document ref (or id), time, currency, and each market quote as sent', () => {
    const refs: OrderRefView[] = [
      ...SAMPLE_ORDER_REFS,
      {
        transactionId: 'tx-2',
        documentRef: null,
        recordedAt: '2026-10-19T10:15:00Z',
        currency: 'EUR',
        market: [
          makePricingQuoteDto({
            source: 'BOT',
            buy: '38.1200',
            sell: '38.6500',
            fetchedAt: '2026-10-19T08:00:00Z',
            stale: false,
          }),
        ],
      },
    ];
    mount(refs);

    const rows = host.querySelectorAll('.order-ref-item');
    expect(rows).toHaveLength(2);

    const first = rows[0];
    expect(first.querySelector('.order-ref-doc')?.textContent).toBe('XC-20261019-000001');
    expect(first.querySelector('.order-ref-time')?.textContent).toBe('2026-10-19T09:00:00Z');
    expect(first.querySelector('.order-ref-currency')?.textContent).toBe('USD');
    const firstQuotes = [...first.querySelectorAll('.order-ref-quote')].map((el) => el.textContent);
    expect(firstQuotes).toHaveLength(2);
    expect(firstQuotes[0]).toContain('TT');
    expect(firstQuotes[0]).toContain('35.10');
    expect(firstQuotes[0]).toContain('35.40');
    expect(firstQuotes[1]).toContain('SR');
    expect(firstQuotes[1]).toContain('35.05');
    expect(firstQuotes[1]).toContain('35.45');

    const second = rows[1];
    expect(second.querySelector('.order-ref-doc')?.textContent).toBe('tx-2');
    expect(second.querySelector('.order-ref-time')?.textContent).toBe('2026-10-19T10:15:00Z');
    expect(second.querySelector('.order-ref-currency')?.textContent).toBe('EUR');
    const secondQuotes = [...second.querySelectorAll('.order-ref-quote')].map((el) => el.textContent);
    expect(secondQuotes).toHaveLength(1);
    expect(secondQuotes[0]).toContain('BOT');
    expect(secondQuotes[0]).toContain('38.1200');
    expect(secondQuotes[0]).toContain('38.6500');
  });

  it('a ref with no market: shows the — text', () => {
    const refs: OrderRefView[] = [
      {
        transactionId: 'tx-no-market',
        documentRef: 'XC-20261019-000002',
        recordedAt: '2026-10-19T11:00:00Z',
        currency: 'JPY',
        market: [],
      },
    ];
    mount(refs);

    const rows = host.querySelectorAll('.order-ref-item');
    expect(rows).toHaveLength(1);
    expect(rows[0].querySelector('.order-ref-market')?.textContent).toBe('—');
    expect(rows[0].querySelectorAll('.order-ref-quote')).toHaveLength(0);
  });

  it('none: renders nothing when refs is empty', () => {
    host = document.createElement('div');
    document.body.appendChild(host);
    const [refs, setRefs] = createSignal<readonly OrderRefView[]>(SAMPLE_ORDER_REFS);
    dispose = render(() => <OrderRefs refs={refs()} />, host);
    expect(host.querySelectorAll('.order-ref-item')).toHaveLength(1);
    setRefs([]);
    expect(host.innerHTML).toBe('');
  });
});
