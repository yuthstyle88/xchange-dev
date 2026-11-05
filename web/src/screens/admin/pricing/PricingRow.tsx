// §9 PricingRow: one currency card, laid out like the shop board's pricing screen (owner 2026-11-05): code + name +
// "ตั้งราคา", then label/value lines (market, shop now, suggested tier 1, status), flags as notes; select box disabled
// when a flag blocks (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import type { JSX } from 'solid-js';
import { For, Show } from 'solid-js';
import type { PricingRowDto } from '../../../shared/api/generated/pricing_board';
import { lang, screenT } from '../../../shared/i18n';
import { RATE } from '../../../shared/ui/bounded';
import * as table from './i18n';
import { BLOCKING_FLAGS, flagLabel, statusLabel, type MarketSourceCode, type PricingFlagCode } from './pricing-labels';
import { currencyName, pctGap, pctText, primarySource, rateAt, rowDigits } from './pricing-format';
import './pricing-row.css';

const t = screenT('admin.pricing', table);

export interface PricingRowProps {
  row: PricingRowDto;
  selected: boolean;
  onToggle(currency: string): void;
  onOpen(currency: string): void;
  /** The card is the one open in the editor (look only). */
  opened?: boolean;
}

const SOURCES: readonly MarketSourceCode[] = ['TT', 'SR', 'BOT'];

/** A buy / sell pair at the currency's decimals, "—" for a missing side. `cls` keeps the cell's class
 *  (`pricing-cell-quote-tt` …). */
export function RatePair(props: {
  cls: string;
  buy: string | null | undefined;
  sell: string | null | undefined;
  digits: number;
  strong?: boolean;
}): JSX.Element {
  return (
    <span class={`pricing-pair ${props.cls}`} classList={{ 'pricing-pair-strong': Boolean(props.strong) }}>
      <span class="pricing-rate-buy">{rateAt(props.buy, props.digits)}</span>
      <span class="pricing-rate-sep" aria-hidden="true">
        /
      </span>
      <span class="pricing-rate-sell">{rateAt(props.sell, props.digits)}</span>
    </span>
  );
}

function arrowPct(g: number | null): string {
  if (g === null) return '—';
  return `${g > 0 ? '▲ ' : g < 0 ? '▼ ' : ''}${pctText(Math.abs(g)).replace('+', '')}`;
}

/** The label/value lines of a currency (market, shop now, suggested tier 1, status) and its notes: the card body,
 *  and the summary at the top of the open editor. */
export function CardLines(props: { row: PricingRowDto }): JSX.Element {
  const quote = (source: MarketSourceCode) => props.row.quotes.find((q) => q.source === source);
  const currentTier1 = () => props.row.published.find((p) => p.tierNo === 1);
  const suggestedTier1 = () => props.row.suggestion?.find((p) => p.tierNo === 1);
  const digits = () => rowDigits(props.row);
  const lead = () => primarySource(props.row);
  const others = () => SOURCES.filter((s) => s !== lead());
  const hasCurrent = () => currentTier1()?.buy != null || currentTier1()?.sell != null;
  const mode = () => props.row.mode;
  const followBlocked = () => props.row.followResult?.status === 'BLOCKED';
  const notes = () => props.row.flags.filter((f) => f !== 'Precision');
  const precision = () => props.row.flags.includes('Precision');

  const marketGap = () => ({
    buy: pctGap(currentTier1()?.buy, quote(lead())?.buy),
    sell: pctGap(currentTier1()?.sell, quote(lead())?.sell),
  });
  const marketGapHigh = () => (marketGap().buy ?? 0) > 0 || (marketGap().sell ?? 0) > 0;
  const suggestedGap = () => ({
    buy: pctGap(suggestedTier1()?.buy, currentTier1()?.buy),
    sell: pctGap(suggestedTier1()?.sell, currentTier1()?.sell),
  });

  const sourceClass = (s: MarketSourceCode) => `pricing-cell-quote-${s.toLowerCase()}`;
  const sourceName = (s: MarketSourceCode) => t(`sourceShort.${s}` as keyof typeof table.th);
  const ModeTag = () => (
    <span class="pricing-row-mode" data-mode={mode()}>
      {t(`mode.${mode()}` as keyof typeof table.th)}
    </span>
  );

  return (
    <>
      <dl class="pricing-lines">
        <div class="pricing-line">
          <dt>{t('cardMarket')}</dt>
          <dd class="pricing-line-value">
            <RatePair cls={sourceClass(lead())} buy={quote(lead())?.buy} sell={quote(lead())?.sell} digits={digits()} />
          </dd>
          <dd class="pricing-line-aside">{sourceName(lead())}</dd>
        </div>
        <div class="pricing-line pricing-line-sub">
          <dt>{t('otherSources')}</dt>
          <dd class="pricing-line-value pricing-line-others">
            <For each={others()}>
              {(s) => (
                <span class="pricing-other-source" data-source={s}>
                  <span class="pricing-other-name">{sourceName(s)}</span>
                  <RatePair cls={sourceClass(s)} buy={quote(s)?.buy} sell={quote(s)?.sell} digits={digits()} />
                </span>
              )}
            </For>
          </dd>
        </div>
        <div class="pricing-line">
          <dt>{t('cardShopNow')}</dt>
          <Show
            when={hasCurrent()}
            fallback={<dd class="pricing-line-value pricing-line-empty pricing-cell-current">{t('noShopPrice')}</dd>}
          >
            <dd class="pricing-line-value">
              <RatePair
                cls="pricing-cell-current"
                buy={currentTier1()?.buy}
                sell={currentTier1()?.sell}
                digits={digits()}
              />
            </dd>
            <dd class="pricing-line-aside">
              <span class="pricing-gap" classList={{ 'pricing-gap-warn': marketGapHigh() }}>
                {t('vsMarket')} {marketGap().buy === null ? '—' : pctText(marketGap().buy!)} /{' '}
                {marketGap().sell === null ? '—' : pctText(marketGap().sell!)}
              </span>
            </dd>
          </Show>
        </div>
        <div class="pricing-line">
          <dt>{t('cardSuggested')}</dt>
          <dd class="pricing-line-value">
            <Show
              when={suggestedTier1()}
              fallback={<span class="pricing-pair pricing-cell-suggested pricing-line-empty">—</span>}
            >
              <RatePair
                cls="pricing-cell-suggested"
                buy={suggestedTier1()?.buy}
                sell={suggestedTier1()?.sell}
                digits={digits()}
                strong
              />
            </Show>
          </dd>
          <Show when={suggestedTier1() && hasCurrent()}>
            <dd class="pricing-line-aside">
              <span class="pricing-gap">
                {arrowPct(suggestedGap().buy)} {arrowPct(suggestedGap().sell)}
              </span>
            </dd>
          </Show>
        </div>
        <div class="pricing-line pricing-line-status pricing-cell-status">
          <dt>{t('cardStatus')}</dt>
          <dd class="pricing-line-value pricing-pills">
            <Show when={mode() === 'FOLLOW'}>
              <span class="pricing-pill-follow" data-follow={props.row.followResult?.status ?? ''}>
                <ModeTag />
                <Show when={followBlocked()}>
                  <span class="pricing-dot" aria-hidden="true">
                    ·
                  </span>
                  <span class="pricing-row-follow" data-follow="BLOCKED">
                    {t('followStatus.BLOCKED')}
                  </span>
                </Show>
              </span>
            </Show>
            <span class="pricing-chip-status" data-status={props.row.status}>
              {statusLabel(props.row.status)}
            </span>
            <Show when={mode() !== 'FOLLOW'}>
              <span class="pricing-mode-tag">
                <ModeTag />
              </span>
            </Show>
          </dd>
        </div>
      </dl>

      <Show when={notes().length > 0}>
        <ul class="pricing-notes pricing-cell-flags">
          <For each={notes()}>
            {(flag) => (
              <li class="pricing-chip-flag" data-flag={flag}>
                {flagLabel(flag)}
              </li>
            )}
          </For>
        </ul>
      </Show>
      <Show when={precision()}>
        <p class="pricing-note-red" data-flag="Precision" title={flagLabel('Precision')}>
          {t('precisionNote', { n: RATE.scale })}
        </p>
      </Show>
    </>
  );
}

export function PricingRow(props: PricingRowProps): JSX.Element {
  const isBlocked = () => props.row.flags.some((f) => BLOCKING_FLAGS.includes(f as PricingFlagCode));
  const name = () => currencyName(props.row.currency, lang());

  return (
    <article
      class="pricing-row pricing-card"
      data-currency={props.row.currency}
      data-status={props.row.status}
      data-enabled={props.row.enabled ? 'true' : 'false'}
      classList={{ 'pricing-card-opened': Boolean(props.opened), 'pricing-card-selected': props.selected }}
    >
      <label class="pricing-cell-select">
        <input
          type="checkbox"
          class="pricing-check"
          name={`select-${props.row.currency}`}
          aria-label={t('selectCurrency', { code: props.row.currency })}
          checked={props.selected}
          disabled={isBlocked()}
          onChange={() => props.onToggle(props.row.currency)}
        />
      </label>

      <div class="pricing-card-body">
        <header class="pricing-card-head pricing-cell-code">
          <div class="pricing-card-title">
            <button type="button" class="pricing-btn-code" onClick={() => props.onOpen(props.row.currency)}>
              {props.row.currency}
            </button>
            <Show when={name()}>
              <span class="pricing-card-name">{name()}</span>
            </Show>
            <Show when={props.row.overridden}>
              <span class="pricing-badge-overridden">{t('overridden')}</span>
            </Show>
          </div>
          <button
            type="button"
            class="pricing-btn-outline"
            name={`set-price-${props.row.currency}`}
            aria-label={`${t('setPrice')} ${props.row.currency}`}
            onClick={() => props.onOpen(props.row.currency)}
          >
            {t('setPrice')}
          </button>
        </header>

        <CardLines row={props.row} />
      </div>
    </article>
  );
}
