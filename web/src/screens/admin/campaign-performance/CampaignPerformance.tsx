// MK-8c (#1646): Admin Campaign Performance screen M04
// (design `docs/design/blueprint/marketing-campaigns.md` D-15, §4.5, §9.1 MK-8; §6.1, §8.x).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  CampaignPerformanceData,
  CampaignPerformanceFilter,
  PerformanceFigure,
  VersionPerformanceRow,
} from '../../../shared/api/generated/campaign_performance';
import { screenT } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
import './campaign-performance.css';

const t = screenT('admin.campaign-performance', table);

const ERROR_KEYS: Record<string, 'error.outOfScope' | 'error.validationFailed'> = {
  outOfScope: 'error.outOfScope',
  validationFailed: 'error.validationFailed',
};

function formatError(err: unknown): string {
  if (err instanceof ApiError) {
    const mapped = ERROR_KEYS[err.code];
    if (mapped) return t(mapped);
    return err.message || err.code;
  }
  return err instanceof Error ? err.message : String(err);
}

function formatMoney(value: string | null | undefined): string {
  if (value === null || value === undefined || value === '') return '—';
  const isDecimal = /^-?\d+(\.\d+)?$/.test(value);
  if (!isDecimal) return value;
  const [intPart, decPart] = value.split('.');
  const isNegative = intPart.startsWith('-');
  const unsignedInt = isNegative ? intPart.slice(1) : intPart;
  const groupedInt = unsignedInt.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${isNegative ? '-' : ''}${groupedInt}${decPart !== undefined ? `.${decPart}` : ''}`;
}

function renderFigure(fig: PerformanceFigure): string {
  if (fig.value !== null && fig.value !== undefined) {
    return formatMoney(fig.value);
  }
  if (fig.unavailable === 'awaitsBroker') {
    return t('awaitsBroker');
  }
  return t('unavailable');
}

export function CampaignPerformance(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [rows, setRows] = createSignal<VersionPerformanceRow[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  let reading = 0;

  async function loadData(branch: string | undefined, from: string, to: string) {
    const mine = ++reading;
    setLoading(true);
    setError(null);
    try {
      const res = await api.view<CampaignPerformanceData, CampaignPerformanceFilter>(
        'campaign_performance',
        {
          branch,
          filter: { from, to },
        },
      );
      if (mine !== reading) return;
      const versions = Array.isArray(res.data?.versions) ? res.data.versions : [];
      setRows(versions);
    } catch (err: unknown) {
      if (mine !== reading) return;
      setRows([]);
      setError(formatError(err));
    } finally {
      if (mine === reading) {
        setLoading(false);
      }
    }
  }

  createEffect(() => {
    if (props.shown && !props.shown()) return;
    const branch = props.branch?.();
    const p = props.period();
    void loadData(branch, p.from, p.to);
  });

  return (
    <div class="xc-cp-screen">
      <Show when={error()}>
        {(errMessage) => (
          <div class="xc-cp-error">
            <span>{errMessage()}</span>
            <button
              type="button"
              class="xc-cp-retry-btn"
              onClick={() => void loadData(props.branch?.(), props.period().from, props.period().to)}
            >
              {t('retry')}
            </button>
          </div>
        )}
      </Show>

      <Show when={loading() && rows().length === 0}>
        <div class="xc-cp-loading">{t('loading')}</div>
      </Show>

      <Show when={!loading() && !error() && rows().length === 0}>
        <div class="xc-cp-empty">{t('empty')}</div>
      </Show>

      <Show when={rows().length > 0}>
        <div class="xc-cp-table-container">
          <table class="xc-cp-table">
            <thead>
              <tr>
                <th>{t('headerCampaign')}</th>
                <th class="text-right">{t('headerVersion')}</th>
                <th class="text-right">{t('headerRedemptions')}</th>
                <th class="text-right">{t('headerPriceCost')}</th>
                <th class="text-right">{t('headerRewardExpense')}</th>
                <th class="text-right">{t('headerVolume')}</th>
                <th class="text-right">{t('headerRealizedMargin')}</th>
                <th class="text-right">{t('headerContribution')}</th>
              </tr>
            </thead>
            <tbody>
              <For each={rows()}>
                {(row) => (
                  <tr data-campaign-id={row.campaignId} data-version-id={row.versionId}>
                    <td class="xc-campaign-name">{row.campaignName}</td>
                    <td class="xc-version text-right">{row.version}</td>
                    <td
                      data-figure="redemptions"
                      data-col="redemptions"
                      class="xc-redemptions text-right"
                    >
                      {row.redemptions}
                    </td>
                    <td
                      data-figure="priceCost"
                      data-col="priceCost"
                      class="xc-price-cost text-right"
                    >
                      {formatMoney(row.priceCostThb)}
                    </td>
                    <td
                      data-figure="rewardExpense"
                      data-col="rewardExpense"
                      class="xc-reward-expense text-right"
                    >
                      {formatMoney(row.rewardExpenseThb)}
                    </td>
                    <td
                      data-figure="volume"
                      data-col="volume"
                      class="xc-volume text-right"
                    >
                      {formatMoney(row.volumeThb)}
                    </td>
                    <td
                      data-figure="realizedMargin"
                      data-col="realizedMargin"
                      class="xc-realized-margin text-right"
                    >
                      {renderFigure(row.realizedMarginThb)}
                    </td>
                    <td
                      data-figure="attributedContribution"
                      data-col="attributedContribution"
                      class="xc-contribution text-right"
                    >
                      {renderFigure(row.attributedContributionThb)}
                    </td>
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
