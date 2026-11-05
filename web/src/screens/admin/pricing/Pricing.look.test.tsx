// Card look of the tier-pricing screen (owner 2026-11-05 "ทำให้หน้าตามันดู มีราคา หน่อย", following the shop board's
// pricing screen). Look only: these pin what the restyle promises and that the actions still send what they sent.
// - recommendation codes print in words (TH / EN), the raw code kept in `title`;
// - rates print without the server's 8-place padding, at the decimals the currency's own figures carry;
// - the folded group "สกุลอื่นที่ TT / Super Rich มี (N)" lists the NotOffered currencies, each card opening the editor;
// - approve (one currency, from the open card) and bulk approve still send one POST /api/pricing/approve.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makePricingBoardData,
  makePricingQuoteDto,
  makePricingRecommendationDto,
  makePricingRowDto,
  makePricingTierRateDto,
  type PricingRowDto,
} from '../../../shared/testing/fixtures/pricing_board';
import { BOARD, boardFetch } from './__fixtures__/pricing-board.fixture';
import { en, th } from './i18n';
import { Pricing } from './Pricing';
import { PricingGrid } from './PricingGrid';
import { PricingRow } from './PricingRow';
import { Recommendations } from './Recommendations';
import { TierEditor } from './TierEditor';
import { ApiClient } from '../../../shared/api/client';
import { rateAt, rowDigits, trimRate } from './pricing-format';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

function mountWith(view: () => ReturnType<typeof PricingRow>) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(view, host);
}

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

const padded = (buy: string, sell: string) =>
  ([1, 2, 3, 4] as const).map((tierNo) => makePricingTierRateDto({ tierNo, buy, sell }));

/** A row the way staging sends it: every rate a numeric(20,8) string. */
const USD8: PricingRowDto = makePricingRowDto({
  ...BOARD.rows[0]!,
  currency: 'USD',
  quotes: [
    makePricingQuoteDto({ source: 'TT', buy: '23.15000000', sell: '23.70000000', fetchedAt: '2026-11-05T03:00:00Z', stale: false }),
    makePricingQuoteDto({ source: 'SR', buy: '23.35000000', sell: '23.50000000', fetchedAt: '2026-11-05T03:00:00Z', stale: false }),
  ],
  published: padded('33.18000000', '33.62000000'),
  suggestion: padded('23.17000000', '23.68000000'),
  flags: [],
});

describe('pricing card look', () => {
  it('L1 recommendation codes print in words, the code kept in title; NotOffered folds into one line', () => {
    const items = [
      makePricingRecommendationDto({ severity: 'High', code: 'SellAboveMarket', currency: 'USD', side: 'SELL' }),
      makePricingRecommendationDto({ severity: 'Low', code: 'MarketMovedSinceUpdate', currency: 'EUR', side: null }),
      makePricingRecommendationDto({ severity: 'Low', code: 'NotOffered', currency: 'AED', side: null }),
      makePricingRecommendationDto({ severity: 'Low', code: 'NotOffered', currency: 'BHD', side: null }),
    ];
    mountWith(() => <Recommendations items={items} />);
    const text = host.textContent ?? '';
    expect(text).toContain(th['rec.SellAboveMarket']);
    expect(text).toContain(th['rec.MarketMovedSinceUpdate']);
    expect(text).toContain(th['rec.NotOffered']);
    expect(text).toContain(th['severity.High']);
    expect(text).toContain(th['side.SELL']);
    for (const raw of ['SellAboveMarket', 'MarketMovedSinceUpdate', 'NotOffered', 'High', 'SELL']) {
      expect(text, `raw "${raw}" is not printed`).not.toContain(raw);
    }
    const sell = host.querySelector('[data-code="SellAboveMarket"] .recommendation-code');
    expect(sell?.getAttribute('title')).toBe('SellAboveMarket');
    const group = host.querySelectorAll('[data-code="NotOffered"]');
    expect(group).toHaveLength(1);
    expect(group[0]!.textContent).toContain('AED');
    expect(group[0]!.textContent).toContain('BHD');

    dispose?.();
    host.remove();
    setLang('en');
    mountWith(() => <Recommendations items={items} />);
    expect(host.textContent).toContain(en['rec.SellAboveMarket']);
    expect(host.textContent).toContain(en['severity.High']);
  });

  it('L2 rates print without the 8-place padding, at the decimals the currency carries', () => {
    mountWith(() => <PricingRow row={USD8} selected={false} onToggle={vi.fn()} onOpen={vi.fn()} />);
    const text = host.textContent ?? '';
    expect(text).not.toMatch(/\d\.\d{8}/);
    expect(host.querySelector('.pricing-cell-quote-tt')?.textContent).toContain('23.15');
    expect(host.querySelector('.pricing-cell-quote-tt')?.textContent).toContain('23.70');
    expect(host.querySelector('.pricing-cell-current')?.textContent).toContain('33.18');
    expect(host.querySelector('.pricing-cell-suggested')?.textContent).toContain('23.68');
    // The decimals come from the figures themselves, never a constant.
    expect(trimRate('23.15000000')).toBe('23.15');
    expect(trimRate('36.00000000')).toBe('36');
    const jpy = makePricingRowDto({ ...USD8, quotes: [], published: [], suggestion: padded('0.20850000', '0.21500000') });
    expect(rowDigits(jpy)).toBe(4);
    expect(rateAt('0.21500000', rowDigits(jpy))).toBe('0.2150');
    expect(rateAt(null, 2)).toBe('—');
  });

  it('L3 the folded group lists NotOffered currencies with their market prices and opens the editor', () => {
    const AED = makePricingRowDto({
      ...USD8,
      currency: 'AED',
      enabled: false,
      quotes: [makePricingQuoteDto({ source: 'TT', buy: '9.10000000', sell: '9.45000000', fetchedAt: '2026-11-05T03:00:00Z', stale: false })],
      published: [],
    });
    const recs = [
      makePricingRecommendationDto({ severity: 'Low', code: 'NotOffered', currency: 'AED', side: null }),
      makePricingRecommendationDto({ severity: 'Low', code: 'NotOffered', currency: 'BHD', side: null }),
    ];
    const onOpen = vi.fn();
    mountWith(() => (
      <PricingGrid rows={[USD8, AED]} selected={new Set()} onToggle={vi.fn()} onOpen={onOpen} recommendations={recs} />
    ));
    const toggle = host.querySelector<HTMLButtonElement>('button[name="toggleDisabled"]');
    expect(toggle?.textContent).toContain(th.unopenedTitle.replace('{count}', '2'));
    expect(host.querySelector('[data-currency="AED"]'), 'folded until opened').toBeNull();
    toggle!.click();
    const group = host.querySelector('#pricing-unopened')!;
    expect(group.querySelector('[data-currency="AED"]')).not.toBeNull();
    expect(group.querySelector('[data-currency="BHD"]')).not.toBeNull();
    expect(group.querySelector('[data-currency="USD"]')).toBeNull();
    expect(group.querySelector('[data-currency="AED"] .pricing-cell-quote-tt')?.textContent).toContain('9.10');
    group.querySelector<HTMLButtonElement>('button[name="set-price-AED"]')!.click();
    expect(onOpen).toHaveBeenCalledWith('AED');
  });

  it('L4 approve from the open card still sends one POST /api/pricing/approve for that currency', async () => {
    const server = boardFetch([BOARD]);
    vi.stubGlobal('fetch', vi.fn(server.fn));
    host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(
      () => <Pricing branch={() => 'MAIN'} period={() => ({ from: '', to: '' })} shown={() => true} />,
      host,
    );
    await flush();
    host.querySelector<HTMLButtonElement>('button[name="set-price-USD"]')!.click();
    await flush();
    const dialog = host.querySelector('[role="dialog"]');
    expect(dialog?.textContent).toContain(th.tierLevel.replace('{n}', '1'));
    expect(dialog?.textContent).toContain(th.bandUpTo.replace('{upper}', '10,000'));
    expect(dialog?.textContent).toContain(th.bandBetween.replace('{lower}', '10,001').replace('{upper}', '50,000'));
    const reason = dialog!.querySelector<HTMLInputElement>('input[name=reason]')!;
    reason.value = 'look check';
    reason.dispatchEvent(new Event('input', { bubbles: true }));
    dialog!.querySelector<HTMLButtonElement>('button[name=approve]')!.click();
    await flush();
    const sent = server.calls.filter((c) => c.method === 'POST' && c.path === '/api/pricing/approve');
    expect(sent).toHaveLength(1);
    expect(sent[0]!.key).toBeTruthy();
    const body = sent[0]!.body as { policyVersionId: string; rows: { currency: string; tiers: unknown[] }[]; reason: string };
    expect(body.policyVersionId).toBe('pv-7');
    expect(body.rows.map((r) => r.currency)).toEqual(['USD']);
    expect(body.rows[0]!.tiers).toHaveLength(4);
    expect(body.reason).toBe('look check');
  });

  it('L5 bulk approve from the cards still sends one POST /api/pricing/approve with the selected rows', async () => {
    const board = makePricingBoardData({ ...BOARD });
    const server = boardFetch([board]);
    vi.stubGlobal('fetch', vi.fn(server.fn));
    host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(
      () => <Pricing branch={() => 'MAIN'} period={() => ({ from: '', to: '' })} shown={() => true} />,
      host,
    );
    await flush();
    host.querySelector<HTMLInputElement>('input[name=select-USD]')!.click();
    await flush();
    host.querySelector<HTMLButtonElement>('button[name=bulkApprove]')!.click();
    await flush();
    const reason = host.querySelector<HTMLInputElement>('.bulk-approve input[name=reason]')!;
    reason.value = 'bulk look';
    reason.dispatchEvent(new Event('input', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('button[name=confirmBulk]')!.click();
    await flush();
    const sent = server.calls.filter((c) => c.method === 'POST' && c.path === '/api/pricing/approve');
    expect(sent).toHaveLength(1);
    expect((sent[0]!.body as { rows: { currency: string }[] }).rows.map((r) => r.currency)).toEqual(['USD']);
  });

  it('L6 a tier box shows the suggestion without the padding; untouched, approve still sends it exactly as the board sent it', async () => {
    const server = boardFetch([BOARD]);
    const api = new ApiClient(server.fn as unknown as typeof fetch);
    mountWith(() => (
      <TierEditor api={api} branch="MAIN" policyVersionId="pv-7" row={USD8} bands={BOARD.bands} onApproved={vi.fn()} />
    ));
    const buy1 = host.querySelector<HTMLInputElement>('input[name=buy-1]')!;
    expect(buy1.value).toBe('23.17');
    expect(host.querySelector('label[for="' + buy1.id + '"]')?.textContent).toBe(th.shopBuy);
    const reason = host.querySelector<HTMLInputElement>('input[name=reason]')!;
    reason.value = 'as suggested';
    reason.dispatchEvent(new Event('input', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('button[name=approve]')!.click();
    await flush();
    const sent = server.calls.filter((c) => c.method === 'POST' && c.path === '/api/pricing/approve');
    expect(sent).toHaveLength(1);
    const tiers = (sent[0]!.body as { rows: { tiers: { buy: string; sell: string }[] }[] }).rows[0]!.tiers;
    expect(tiers[0]).toMatchObject({ buy: '23.17000000', sell: '23.68000000' });
  });
});
