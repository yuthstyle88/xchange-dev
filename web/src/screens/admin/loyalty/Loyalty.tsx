// LY-6-FE (#2129): EXECUTIVE loyalty rule-version screen
// (design `docs/design/blueprint/membership-points.md` §4.6, §6.4, §9.1 rows LY-6-FE and LY-6-view).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type {
  LoyaltyRuleVersionsData,
  PointRuleVersionItem,
  TierRuleVersionItem,
} from '../../../shared/api/generated/loyalty_rule_versions';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';

const t = screenT('admin.loyalty', table);

const STATUS_KEYS: Record<string, keyof typeof table.th> = {
  DRAFT: 'status.draft',
  PENDING_APPROVAL: 'status.pending_approval',
  APPROVED: 'status.approved',
  REJECTED: 'status.rejected',
};

function statusLabel(status: string): string {
  const key = STATUS_KEYS[status];
  return key ? t(key) : status;
}

export function Loyalty(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [pointVersions, setPointVersions] = createSignal<PointRuleVersionItem[]>([]);
  const [tierVersions, setTierVersions] = createSignal<TierRuleVersionItem[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  let reading = 0;
  async function loadVersions() {
    const mine = ++reading;
    setLoading(true);
    setError(null);
    try {
      const res = await api.viewBare<LoyaltyRuleVersionsData>('loyalty_rule_versions');
      if (mine !== reading) return;
      setPointVersions(Array.isArray(res?.pointVersions) ? res.pointVersions : []);
      setTierVersions(Array.isArray(res?.tierVersions) ? res.tierVersions : []);
    } catch (err: unknown) {
      if (mine !== reading) return;
      setPointVersions([]);
      setTierVersions([]);
      setError(wordedError(err));
    } finally {
      if (mine === reading) {
        setLoading(false);
      }
    }
  }

  createEffect(() => {
    if (props.shown && !props.shown()) return;
    void loadVersions();
  });

  return (
    <div class="xc-loyalty-screen">
      <Show when={error()}>
        {(msg) => (
          <div role="alert" class="xc-loyalty-error">
            {msg()}
          </div>
        )}
      </Show>

      <Show when={loading() && pointVersions().length === 0 && tierVersions().length === 0}>
        <div class="xc-loyalty-loading">{t('loading')}</div>
      </Show>

      <section class="xc-loyalty-section">
        <h4>{t('pointVersionsTitle')}</h4>
        <table class="xc-loyalty-table">
          <thead>
            <tr>
              <th>{t('headerVersion')}</th>
              <th>{t('headerStatus')}</th>
              <th>{t('headerBasis')}</th>
              <th>{t('headerPointsRatio')}</th>
              <th>{t('headerRounding')}</th>
              <th>{t('headerMinimumBasisAmount')}</th>
              <th>{t('headerExpiryMonths')}</th>
              <th>{t('headerValidFrom')}</th>
              <th>{t('headerValidTo')}</th>
              <th>{t('headerActions')}</th>
            </tr>
          </thead>
          <tbody>
            <For each={pointVersions()}>
              {(row) => (
                <tr
                  data-version-id={row.versionId}
                  data-kind="POINT"
                  data-version={String(row.version)}
                  data-status={row.status}
                >
                  <td>{row.version}</td>
                  <td>{statusLabel(row.status)}</td>
                  <td>{row.basis}</td>
                  <td>{row.pointsRatio}</td>
                  <td>{row.rounding}</td>
                  <td>{row.minimumBasisAmount}</td>
                  <td>{row.expiryMonths ?? t('dash')}</td>
                  <td>{row.validFrom}</td>
                  <td>{row.validTo ?? t('dash')}</td>
                  <td>
                    <Show when={row.status === 'DRAFT'}>
                      <button type="button" data-action="submit">
                        {t('submit')}
                      </button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </section>

      <section class="xc-loyalty-section">
        <h4>{t('tierVersionsTitle')}</h4>
        <table class="xc-loyalty-table">
          <thead>
            <tr>
              <th>{t('headerVersion')}</th>
              <th>{t('headerStatus')}</th>
              <th>{t('headerRollingMonths')}</th>
              <th>{t('headerValidFrom')}</th>
              <th>{t('headerValidTo')}</th>
              <th>{t('headerActions')}</th>
            </tr>
          </thead>
          <tbody>
            <For each={tierVersions()}>
              {(row) => (
                <tr
                  data-version-id={row.versionId}
                  data-kind="TIER"
                  data-version={String(row.version)}
                  data-status={row.status}
                >
                  <td>{row.version}</td>
                  <td>{statusLabel(row.status)}</td>
                  <td>{row.rollingMonths}</td>
                  <td>{row.validFrom}</td>
                  <td>{row.validTo ?? t('dash')}</td>
                  <td>
                    <Show when={row.status === 'DRAFT'}>
                      <button type="button" data-action="submit">
                        {t('submit')}
                      </button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </section>
    </div>
  );
}
