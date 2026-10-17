// LY-6-FE (#2129): EXECUTIVE loyalty rule-version screen
// (design `docs/design/blueprint/membership-points.md` §4.6, §6.4, §9.1 rows LY-6-FE and LY-6-view).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  LoyaltyRuleDraftInput,
  LoyaltyRuleDraftOutput,
} from '../../../shared/api/generated/loyalty_rule_draft';
import type {
  LoyaltyRuleSubmitInput,
  LoyaltyRuleSubmitOutput,
} from '../../../shared/api/generated/loyalty_rule_submit';
import type {
  LoyaltyRuleVersionsData,
  PointRuleVersionItem,
  TierRuleVersionItem,
} from '../../../shared/api/generated/loyalty_rule_versions';
import { screenT, t as sharedT } from '../../../shared/i18n';
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

function loyaltyError(err: unknown): string {
  if (err instanceof ApiError && err.code === 'loyaltyUnavailable') {
    return sharedT('error.loyaltyUnavailable');
  }
  return wordedError(err);
}

function toBangkokRfc3339(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (/^\d{4}-\d{2}-\d{2}$/.test(trimmed)) {
    return `${trimmed}T00:00:00+07:00`;
  }
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(trimmed)) {
    return `${trimmed}:00+07:00`;
  }
  return trimmed;
}

function parseOptionalInt(raw: string): number | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number.parseInt(trimmed, 10);
  return Number.isNaN(n) ? null : n;
}

export function Loyalty(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const draftWrite = keyedWrite<LoyaltyRuleDraftInput, LoyaltyRuleDraftOutput>(
    api,
    'POST',
    '/api/v3/commands/loyalty_rule_draft',
  );
  const submitWrite = keyedWrite<LoyaltyRuleSubmitInput, LoyaltyRuleSubmitOutput>(
    api,
    'POST',
    '/api/v3/commands/loyalty_rule_submit',
  );

  const [pointVersions, setPointVersions] = createSignal<PointRuleVersionItem[]>([]);
  const [tierVersions, setTierVersions] = createSignal<TierRuleVersionItem[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // POINT draft form signals (no business threshold or ratio is hard-coded)
  const [basis, setBasis] = createSignal('SETTLE_THB');
  const [pointsRatio, setPointsRatio] = createSignal('');
  const [rounding, setRounding] = createSignal('FLOOR');
  const [minimumBasisAmount, setMinimumBasisAmount] = createSignal('');
  const [expiryMonths, setExpiryMonths] = createSignal('');
  const [validFrom, setValidFrom] = createSignal('');
  const [validTo, setValidTo] = createSignal('');

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
      setError(loyaltyError(err));
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

  async function handleSavePointDraft() {
    const nextVersion = pointVersions().reduce((max, v) => Math.max(max, v.version), 0) + 1;
    const fields: Record<string, unknown> = {
      version: nextVersion,
      basis: basis(),
      pointsRatio: pointsRatio().trim(),
      rounding: rounding(),
      minimumBasisAmount: minimumBasisAmount().trim(),
      expiryMonths: parseOptionalInt(expiryMonths()),
      validFrom: toBangkokRfc3339(validFrom()) ?? '',
      validTo: toBangkokRfc3339(validTo()),
    };

    setError(null);
    try {
      await draftWrite.send({ kind: 'POINT', fields });
      await loadVersions();
    } catch (err: unknown) {
      setError(loyaltyError(err));
    }
  }

  async function handleSubmit(versionId: string) {
    setError(null);
    try {
      await submitWrite.send({ versionId });
      await loadVersions();
    } catch (err: unknown) {
      setError(loyaltyError(err));
    }
  }

  return (
    <div>
      <Show when={error()}>
        {(msg) => (
          <div role="alert">
            {msg()}
          </div>
        )}
      </Show>

      <Show when={loading() && pointVersions().length === 0 && tierVersions().length === 0}>
        <div>{t('loading')}</div>
      </Show>

      <section>
        <h4>{t('pointVersionsTitle')}</h4>
        <table>
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
                      <button
                        type="button"
                        data-action="submit"
                        onClick={() => void handleSubmit(row.versionId)}
                      >
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

      <section data-form="point-draft">
        <h4>{t('draftPointTitle')}</h4>
        <div class="xc-form-grid">
          <div class="xc-form-group">
            <label>{t('fieldBasis')}</label>
            <select
              name="basis"
              value={basis()}
              onInput={(e) => setBasis(e.currentTarget.value)}
              onChange={(e) => setBasis(e.currentTarget.value)}
            >
              <option value="SETTLE_THB">{t('basisSettleThb')}</option>
              <option value="THB_AMOUNT">{t('basisThbAmount')}</option>
              <option value="REALIZED_MARGIN_THB">{t('basisRealizedMarginThb')}</option>
            </select>
          </div>

          <div class="xc-form-group">
            <label>{t('fieldPointsRatio')}</label>
            <input
              type="text"
              name="pointsRatio"
              value={pointsRatio()}
              onInput={(e) => setPointsRatio(e.currentTarget.value)}
              onChange={(e) => setPointsRatio(e.currentTarget.value)}
            />
          </div>

          <div class="xc-form-group">
            <label>{t('fieldRounding')}</label>
            <select
              name="rounding"
              value={rounding()}
              onInput={(e) => setRounding(e.currentTarget.value)}
              onChange={(e) => setRounding(e.currentTarget.value)}
            >
              <option value="FLOOR">{t('roundingFloor')}</option>
              <option value="HALF_UP">{t('roundingHalfUp')}</option>
            </select>
          </div>

          <div class="xc-form-group">
            <label>{t('fieldMinimumBasisAmount')}</label>
            <input
              type="text"
              name="minimumBasisAmount"
              value={minimumBasisAmount()}
              onInput={(e) => setMinimumBasisAmount(e.currentTarget.value)}
              onChange={(e) => setMinimumBasisAmount(e.currentTarget.value)}
            />
          </div>

          <div class="xc-form-group">
            <label>{t('fieldExpiryMonths')}</label>
            <input
              type="text"
              name="expiryMonths"
              value={expiryMonths()}
              onInput={(e) => setExpiryMonths(e.currentTarget.value)}
              onChange={(e) => setExpiryMonths(e.currentTarget.value)}
            />
          </div>

          <div class="xc-form-group">
            <label>{t('fieldValidFrom')}</label>
            <input
              type="date"
              name="validFrom"
              value={validFrom()}
              onInput={(e) => setValidFrom(e.currentTarget.value)}
              onChange={(e) => setValidFrom(e.currentTarget.value)}
            />
          </div>

          <div class="xc-form-group">
            <label>{t('fieldValidTo')}</label>
            <input
              type="date"
              name="validTo"
              value={validTo()}
              onInput={(e) => setValidTo(e.currentTarget.value)}
              onChange={(e) => setValidTo(e.currentTarget.value)}
            />
          </div>
        </div>
        <button
          type="button"
          data-action="save-draft"
          onClick={() => void handleSavePointDraft()}
        >
          {t('saveDraft')}
        </button>
      </section>

      <section>
        <h4>{t('tierVersionsTitle')}</h4>
        <table>
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
                      <button
                        type="button"
                        data-action="submit"
                        onClick={() => void handleSubmit(row.versionId)}
                      >
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
