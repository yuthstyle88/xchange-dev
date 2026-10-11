import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { ApiClient } from '../../../shared/api/client';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import { RateFreshness } from '../../../shared/ui/RateFreshness';
import type { StaffCtx } from '../cards';
import { th, en } from './i18n';
import './rates.css';

export interface RatesCardProps {
  ctx: StaffCtx;
}

export function RatesCard(props: RatesCardProps): JSX.Element {
  const t = screenT('staff.rates', { th, en });
  const [rates, setRates] = createSignal<PublishedRatesData | null>(null);

  const load = async () => {
    try {
      const api = new ApiClient();
      const res = await api.view<PublishedRatesData>('staff_published_rates', {
        branch: props.ctx.me.branchId,
      });
      if (res.data) {
        setRates(res.data);
      }
    } catch {
      // view error
    }
  };

  onMount(() => {
    void load();
  });

  return (
    <div class="xc-rates-card">
      <div class="xc-rates-header">
        <div class="xc-rates-title-group">
          <h2 class="xc-rates-title">{t('title')}</h2>
          <Show when={rates()?.publication}>
            {(pub) => (
              <span class="xc-rates-version">
                {t('version', { version: pub().version })}
              </span>
            )}
          </Show>
        </div>
      </div>

      <RateFreshness
        stale={rates()?.stale ?? false}
        sourceDate={rates()?.sourceDate}
        publishedAt={rates()?.publication?.publishedAt}
      />

      <Show
        when={(rates()?.lines ?? []).length > 0}
        fallback={
          <Show when={rates() !== null}>
            <p class="xc-rates-empty">{t('empty')}</p>
          </Show>
        }
      >
        <div class="xc-rates-table-container">
          <table class="xc-rates-table">
            <thead>
              <tr class="xc-rates-head-row">
                <th class="xc-rates-th-currency">{t('currency')}</th>
                <th class="xc-rates-th-unit">{t('unit')}</th>
                <th class="xc-rates-th-buy">{t('buy')}</th>
                <th class="xc-rates-th-sell">{t('sell')}</th>
              </tr>
            </thead>
            <tbody>
              <For each={rates()?.lines ?? []}>
                {(line) => (
                  <tr class="xc-rates-row">
                    <td class="xc-rates-td-currency">{line.currency}</td>
                    <td class="xc-rates-td-unit">{line.unitSize}</td>
                    <td class="xc-rates-td-buy">{line.buyRate !== null ? line.buyRate : t('unavailable')}</td>
                    <td class="xc-rates-td-sell">{line.sellRate !== null ? line.sellRate : t('unavailable')}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}
