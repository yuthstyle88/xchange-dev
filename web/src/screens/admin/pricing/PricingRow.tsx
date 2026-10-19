// §9 PricingRow: code, TT / SR / BoT, current shop rate, suggested tier 1, status chip, flags, select box (disabled when a flag blocks) (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import type { JSX } from 'solid-js';
import { For, Show } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';
import { BLOCKING_FLAGS, type MarketSourceCode, type PricingRowView } from './pricing-board.types';
import { flagLabel, statusLabel } from './pricing-labels';
import './pricing-row.css';

const t = screenT('admin.pricing', table);

export interface PricingRowProps {
  row: PricingRowView;
  selected: boolean;
  onToggle(currency: string): void;
  onOpen(currency: string): void;
}

function rate(val: string | null | undefined): string {
  return val != null && val !== '' ? val : '—';
}

export function PricingRow(props: PricingRowProps): JSX.Element {
  const isBlocked = () => props.row.flags.some((f) => BLOCKING_FLAGS.includes(f));
  const quote = (source: MarketSourceCode) => props.row.quotes.find((q) => q.source === source);
  const currentTier1 = () => props.row.published.find((p) => p.tierNo === 1);
  const suggestedTier1 = () => props.row.suggestion?.tiers.find((p) => p.tierNo === 1);

  return (
    <tr class="pricing-row" data-currency={props.row.currency}>
      <td class="pricing-cell-select">
        <input
          type="checkbox"
          name={`select-${props.row.currency}`}
          checked={props.selected}
          disabled={isBlocked()}
          onChange={() => props.onToggle(props.row.currency)}
        />
      </td>
      <td class="pricing-cell-code">
        <button
          type="button"
          class="pricing-btn-code"
          onClick={() => props.onOpen(props.row.currency)}
        >
          {props.row.currency}
        </button>
        <Show when={props.row.overridden}>
          <span class="pricing-badge-overridden">{t('overridden')}</span>
        </Show>
      </td>
      <td class="pricing-cell-quote-tt">
        <span class="pricing-rate-buy">{rate(quote('TT')?.buy)}</span>
        <span class="pricing-rate-sep">/</span>
        <span class="pricing-rate-sell">{rate(quote('TT')?.sell)}</span>
      </td>
      <td class="pricing-cell-quote-sr">
        <span class="pricing-rate-buy">{rate(quote('SR')?.buy)}</span>
        <span class="pricing-rate-sep">/</span>
        <span class="pricing-rate-sell">{rate(quote('SR')?.sell)}</span>
      </td>
      <td class="pricing-cell-quote-bot">
        <span class="pricing-rate-buy">{rate(quote('BOT')?.buy)}</span>
        <span class="pricing-rate-sep">/</span>
        <span class="pricing-rate-sell">{rate(quote('BOT')?.sell)}</span>
      </td>
      <td class="pricing-cell-current">
        <span class="pricing-rate-buy">{rate(currentTier1()?.buy)}</span>
        <span class="pricing-rate-sep">/</span>
        <span class="pricing-rate-sell">{rate(currentTier1()?.sell)}</span>
      </td>
      <td class="pricing-cell-suggested">
        <span class="pricing-rate-buy">{rate(suggestedTier1()?.buy)}</span>
        <span class="pricing-rate-sep">/</span>
        <span class="pricing-rate-sell">{rate(suggestedTier1()?.sell)}</span>
      </td>
      <td class="pricing-cell-status">
        <span class="pricing-chip-status">{statusLabel(props.row.status)}</span>
      </td>
      <td class="pricing-cell-flags">
        <For each={props.row.flags}>
          {(flag) => <span class="pricing-chip-flag">{flagLabel(flag)}</span>}
        </For>
      </td>
    </tr>
  );
}
