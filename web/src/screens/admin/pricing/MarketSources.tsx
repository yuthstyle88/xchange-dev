// §9 MarketSources: one chip per source: name, age, stale, error; refresh if allowed (FE pricing-board, #564, #3760; docs/design/blueprint/pricing-tiers.md §9).
import { For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { en, th } from './i18n';
import type { MarketSourceCode, PricingSource } from './pricing-board.types';
import './market-sources.css';

export interface MarketSourcesProps {
  sources: readonly PricingSource[];
  /** The caller holds `MarketRatesRefresh` (§8). */
  canRefresh: boolean;
  onRefresh(source: MarketSourceCode): void;
}

export function MarketSources(props: MarketSourcesProps): JSX.Element {
  const t = screenT('admin.pricing', { th, en });

  return (
    <div class="xc-market-sources">
      <For each={props.sources}>
        {(s) => (
          <div class="xc-market-source-chip" data-source={s.source}>
            <span class="xc-source-name">{s.name}</span>
            <Show when={s.fetchedAt}>
              <span class="xc-source-time">{new Date(s.fetchedAt!).toLocaleString()}</span>
            </Show>
            <Show when={s.stale}>
              <span class="xc-source-stale">{t('stale')}</span>
            </Show>
            <Show when={s.error}>
              <span class="xc-source-error">{t('sourceError')}: {s.error}</span>
            </Show>
            <Show when={props.canRefresh && s.source !== 'BOT'}>
              <button
                type="button"
                class="xc-source-refresh-btn"
                name={`refresh-${s.source}`}
                onClick={() => props.onRefresh(s.source)}
              >
                {t('refresh')}
              </button>
            </Show>
          </div>
        )}
      </For>
    </div>
  );
}
