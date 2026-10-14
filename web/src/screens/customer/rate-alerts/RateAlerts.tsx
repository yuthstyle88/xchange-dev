// AL-5 customer rate-alerts screen (design rate-alerts.md §4.4, §9.1 C11).
import {
  createSignal,
  onCleanup,
  onMount,
  For,
  Show,
  type JSX,
} from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  CreateRateAlertInput,
  MyRateAlertItem,
  MyRateAlertsData,
} from '../../../shared/api/generated/my_rate_alerts';
import type {
  PublicBranchDto,
  PublicBranchesData,
} from '../../../shared/api/public/public_branches';
import { errorText, fill, locale, screenT, t as sharedT } from '../../../shared/i18n';
import { MoneyText } from '../../../shared/ui/MoneyText';
import * as table from './i18n';
import './rate-alerts.css';

const t = screenT('customer.rate-alerts', table);

export interface RateAlertsScreenProps {
  /** Injected ApiClient */
  client?: ApiClient;
  /** Injected fetch function for testing */
  fetch?: (input: string, init?: RequestInit) => Promise<Response>;
  /** Organisation code override; if omitted, fetched from /api/public/host */
  org?: string;
  /** Default branch code */
  defaultBranch?: string;
  /** Polling interval in ms (optional) */
  pollIntervalMs?: number;
}

function generateIdempotencyKey(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return 'key-' + Date.now() + '-' + Math.random().toString(36).slice(2);
}

export function RateAlertsScreen(props: RateAlertsScreenProps): JSX.Element {
  const fetchFn = props.fetch ?? ((input, init) => fetch(input, init));
  const api = props.client ?? new ApiClient(fetchFn);

  const [alerts, setAlerts] = createSignal<MyRateAlertItem[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal<string | null>(null);

  // Branches list from public API
  const [branches, setBranches] = createSignal<PublicBranchDto[]>([]);
  const [branchesError, setBranchesError] = createSignal<string | null>(null);
  const [branch, setBranch] = createSignal(props.defaultBranch ?? '');

  // Create form state
  const [currency, setCurrency] = createSignal('USD');
  const [customerSide, setCustomerSide] = createSignal<'CUSTOMER_BUY' | 'CUSTOMER_SELL'>('CUSTOMER_BUY');
  const [condition, setCondition] = createSignal<'ABOVE' | 'BELOW'>('ABOVE');
  const [target, setTarget] = createSignal('');
  const [expiresAt, setExpiresAt] = createSignal('');
  const [createKey, setCreateKey] = createSignal(generateIdempotencyKey());
  const [isCreating, setIsCreating] = createSignal(false);
  const [createError, setCreateError] = createSignal<string | null>(null);

  // Action status/error
  const [actionError, setActionError] = createSignal<string | null>(null);

  let sinceVersion: number | undefined = undefined;
  let timerId: ReturnType<typeof setInterval> | undefined = undefined;

  const formatErrorMsg = (err: unknown): string => {
    const fallback = sharedT('error.serverError');
    if (err instanceof ApiError) {
      if (err.code === 'alertLimitReached') return t('alertLimitReached');
      if (err.code === 'alertSettingsMissing') return t('alertSettingsMissing');
      if (err.code === 'idempotencyConflict') return t('idempotencyConflict');
      if (err.code === 'staleVersion') return t('staleVersion');
      if (err.code === 'validationFailed') return t('validationFailed');
      if (err.code && err.code !== 'httpError') {
        const text = errorText(err.code, fallback);
        if (text && text !== err.code) return text;
      }
      return fallback;
    }
    if (err && typeof err === 'object' && 'code' in err) {
      const c = String((err as { code: unknown }).code);
      if (c === 'alertLimitReached') return t('alertLimitReached');
      if (c === 'alertSettingsMissing') return t('alertSettingsMissing');
      if (c === 'idempotencyConflict') return t('idempotencyConflict');
      if (c === 'staleVersion') return t('staleVersion');
      if (c === 'validationFailed') return t('validationFailed');
      if (c && c !== 'httpError') {
        const text = errorText(c, fallback);
        if (text && text !== c) return text;
      }
      return fallback;
    }
    return fallback;
  };

  const loadBranches = async () => {
    try {
      let org = props.org;
      if (!org) {
        const res = await fetchFn('/api/public/host');
        if (!res.ok) {
          let errJson: any = null;
          try {
            errJson = await res.json();
          } catch {
            // ignore
          }
          if (errJson?.error) {
            throw new ApiError({
              code: errJson.error.code ?? 'serverError',
              message: errJson.error.message ?? '',
              status: res.status,
            });
          }
          throw new ApiError({
            code: 'httpError',
            message: res.statusText,
            status: res.status,
          });
        }
        const json = (await res.json()) as { org?: string };
        if (json && typeof json.org === 'string' && json.org.length > 0) {
          org = json.org;
        }
      }
      if (org) {
        const branchesUrl = `/api/public/orgs/${encodeURIComponent(org)}/branches`;
        const bRes = await fetchFn(branchesUrl);
        if (!bRes.ok) {
          let errJson: any = null;
          try {
            errJson = await bRes.json();
          } catch {
            // ignore
          }
          if (errJson?.error) {
            throw new ApiError({
              code: errJson.error.code ?? 'serverError',
              message: errJson.error.message ?? '',
              status: bRes.status,
            });
          }
          throw new ApiError({
            code: 'httpError',
            message: bRes.statusText,
            status: bRes.status,
          });
        }
        const bJson = (await bRes.json()) as PublicBranchesData;
        const list = bJson.branches ?? [];
        setBranches(list);
        setBranchesError(null);
        if (list.length > 0 && !branch()) {
          setBranch(list[0].code);
        }
      }
    } catch (err: unknown) {
      setBranchesError(formatErrorMsg(err));
    }
  };

  const loadAlerts = async (forceFull = false) => {
    if (forceFull) {
      sinceVersion = undefined;
    }
    if (alerts().length === 0) {
      setLoading(true);
    }
    setLoadError(null);
    try {
      const answer = await api.view<MyRateAlertsData>('my_rate_alerts', { since: sinceVersion });
      if (answer.changed && answer.data) {
        setAlerts(answer.data.alerts ?? []);
        sinceVersion = answer.version;
      } else if (sinceVersion === undefined && answer.data) {
        setAlerts(answer.data.alerts ?? []);
        sinceVersion = answer.version;
      }
      // If changed === false, keep alerts as is per §8
    } catch (err: unknown) {
      if (alerts().length === 0) {
        setLoadError(formatErrorMsg(err));
      }
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    void loadBranches();
    void loadAlerts();
    if (props.pollIntervalMs && props.pollIntervalMs > 0) {
      timerId = setInterval(() => {
        void loadAlerts(false);
      }, props.pollIntervalMs);
    }
  });

  onCleanup(() => {
    if (timerId !== undefined) {
      clearInterval(timerId);
    }
  });

  const handleRetry = () => {
    void loadBranches();
    void loadAlerts(true);
  };

  const handleCreate = async (e: Event) => {
    e.preventDefault();
    if (isCreating()) return;

    const trimmedBranch = branch().trim();
    const trimmedTarget = target().trim();
    if (!trimmedBranch || !trimmedTarget) {
      setCreateError(t('validationFailed'));
      return;
    }

    setIsCreating(true);
    setCreateError(null);
    setActionError(null);

    const input: CreateRateAlertInput = {
      branch: trimmedBranch,
      currency: currency().trim().toUpperCase(),
      customerSide: customerSide(),
      condition: condition(),
      target: trimmedTarget,
    };
    if (expiresAt().trim()) {
      input.expiresAt = expiresAt().trim();
    }

    try {
      await api.command('POST', '/api/app/rate-alerts', input, createKey());
      // On success: clear form inputs, rotate key, and refresh
      setTarget('');
      setExpiresAt('');
      setCreateKey(generateIdempotencyKey());
      await loadAlerts(true);
    } catch (err: unknown) {
      // On error (including alertLimitReached): keep form inputs, keep key for retry
      setCreateError(formatErrorMsg(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        void loadAlerts(true);
      }
    } finally {
      setIsCreating(false);
    }
  };

  const handlePause = async (alert: MyRateAlertItem) => {
    setActionError(null);
    const key = generateIdempotencyKey();
    try {
      await api.command(
        'POST',
        `/api/app/rate-alerts/${encodeURIComponent(alert.id)}/pause`,
        { expectedVersion: alert.version },
        key,
      );
      await loadAlerts(true);
    } catch (err: unknown) {
      setActionError(formatErrorMsg(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        void loadAlerts(true);
      }
    }
  };

  const handleResume = async (alert: MyRateAlertItem) => {
    setActionError(null);
    const key = generateIdempotencyKey();
    try {
      await api.command(
        'POST',
        `/api/app/rate-alerts/${encodeURIComponent(alert.id)}/resume`,
        { expectedVersion: alert.version },
        key,
      );
      await loadAlerts(true);
    } catch (err: unknown) {
      setActionError(formatErrorMsg(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        void loadAlerts(true);
      }
    }
  };

  const handleDelete = async (alert: MyRateAlertItem) => {
    setActionError(null);
    const key = generateIdempotencyKey();
    try {
      await api.command(
        'POST',
        `/api/app/rate-alerts/${encodeURIComponent(alert.id)}/delete`,
        { expectedVersion: alert.version },
        key,
      );
      await loadAlerts(true);
    } catch (err: unknown) {
      setActionError(formatErrorMsg(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        void loadAlerts(true);
      }
    }
  };

  const formatUnitSize = (u: string): string => u.replace(/\.0+$/, '');

  const formatRateTime = (isoString: string): string => {
    try {
      return new Date(isoString).toLocaleString(locale(), {
        timeZone: 'Asia/Bangkok',
      });
    } catch {
      return isoString;
    }
  };

  return (
    <div class="xc-rate-alerts-screen">
      <header class="xc-alerts-header">
        <div class="xc-alerts-title-group">
          <h1>{t('title')}</h1>
          <p class="xc-alerts-subtitle">{t('subtitle')}</p>
        </div>
      </header>

      {/* Create Alert Section */}
      <section class="xc-create-alert-section" aria-labelledby="create-alert-heading">
        <h2 id="create-alert-heading">{t('createTitle')}</h2>
        <form class="xc-create-alert-form" onSubmit={handleCreate}>
          <div class="xc-form-group">
            <label class="xc-form-label" for="alert-branch">{t('branch')}</label>
            <select
              id="alert-branch"
              class="xc-form-select"
              value={branch()}
              onChange={(e) => setBranch(e.currentTarget.value)}
              required
            >
              <For each={branches()}>
                {(b) => (
                  <option value={b.code}>
                    {b.name ? `${b.name} (${b.code})` : b.code}
                  </option>
                )}
              </For>
            </select>
            <Show when={branchesError()}>
              <div class="xc-form-error" role="alert">
                {branchesError()}
              </div>
            </Show>
          </div>

          <div class="xc-form-group">
            <label class="xc-form-label" for="alert-currency">{t('currency')}</label>
            <input
              id="alert-currency"
              class="xc-form-input"
              type="text"
              value={currency()}
              onInput={(e) => setCurrency(e.currentTarget.value.toUpperCase())}
              required
            />
          </div>

          <div class="xc-form-group">
            <label class="xc-form-label" for="alert-side">{t('side')}</label>
            <select
              id="alert-side"
              class="xc-form-select"
              value={customerSide()}
              onChange={(e) => setCustomerSide(e.currentTarget.value as 'CUSTOMER_BUY' | 'CUSTOMER_SELL')}
            >
              <option value="CUSTOMER_BUY">{t('sideBuy')}</option>
              <option value="CUSTOMER_SELL">{t('sideSell')}</option>
            </select>
          </div>

          <div class="xc-form-group">
            <label class="xc-form-label" for="alert-condition">{t('condition')}</label>
            <select
              id="alert-condition"
              class="xc-form-select"
              value={condition()}
              onChange={(e) => setCondition(e.currentTarget.value as 'ABOVE' | 'BELOW')}
            >
              <option value="ABOVE">{t('conditionAbove')}</option>
              <option value="BELOW">{t('conditionBelow')}</option>
            </select>
          </div>

          <div class="xc-form-group">
            <label class="xc-form-label" for="alert-target">{t('target')}</label>
            <input
              id="alert-target"
              class="xc-form-input"
              type="text"
              inputMode="decimal"
              placeholder={t('targetPlaceholder')}
              value={target()}
              onInput={(e) => setTarget(e.currentTarget.value)}
              required
            />
          </div>

          <div class="xc-form-group">
            <label class="xc-form-label" for="alert-expires">{t('expiresAt')}</label>
            <input
              id="alert-expires"
              class="xc-form-input"
              type="text"
              value={expiresAt()}
              onInput={(e) => setExpiresAt(e.currentTarget.value)}
            />
          </div>

          <Show when={createError()}>
            <div class="xc-form-error" role="alert">
              {createError()}
            </div>
          </Show>

          <div class="xc-form-actions">
            <button
              type="submit"
              class="xc-btn-primary"
              disabled={isCreating()}
            >
              {isCreating() ? t('creating') : t('submitCreate')}
            </button>
          </div>
        </form>
      </section>

      {/* Action error banner */}
      <Show when={actionError()}>
        <div class="xc-form-error" role="alert" style={{ "margin-bottom": "1rem" }}>
          {actionError()}
        </div>
      </Show>

      {/* Loading State */}
      <Show when={loading() && alerts().length === 0}>
        <div class="xc-alerts-loading" role="status">
          {t('loading')}
        </div>
      </Show>

      {/* Error State */}
      <Show when={!loading() && loadError() && alerts().length === 0}>
        <div class="xc-alerts-error" role="alert">
          <p>{loadError()}</p>
          <button type="button" class="xc-alerts-retry-btn" onClick={handleRetry}>
            {t('retry')}
          </button>
        </div>
      </Show>

      {/* Empty State */}
      <Show when={!loading() && !loadError() && alerts().length === 0}>
        <div class="xc-alerts-empty">
          {t('empty')}
        </div>
      </Show>

      {/* Alerts List */}
      <Show when={alerts().length > 0}>
        <div class="xc-alerts-list">
          <For each={alerts()}>
            {(alert) => {
              const sideLabel = () => alert.customerSide === 'CUSTOMER_BUY' ? t('sideBuy') : t('sideSell');
              const conditionLabel = () => alert.condition === 'ABOVE' ? t('conditionAbove') : t('conditionBelow');
              const unitLabel = () => fill(t('unitPer'), {
                unitSize: formatUnitSize(alert.unitSize),
                unit: formatUnitSize(alert.unitSize),
              });

              return (
                <article class="xc-alert-card" data-alert-id={alert.id}>
                  <div class="xc-alert-card-header">
                    <div class="xc-alert-currency-side">
                      <span class="xc-alert-currency">{alert.currency}</span>
                      <span class="xc-alert-side">{sideLabel()}</span>
                      <span class="xc-alert-unit">({unitLabel()})</span>
                      <span class="xc-alert-branch-badge">{alert.branch}</span>
                    </div>

                    <span class={alert.enabled ? "xc-badge xc-badge-active" : "xc-badge xc-badge-paused"}>
                      {alert.enabled ? t('statusActive') : t('statusPaused')}
                    </span>
                  </div>

                  <div class="xc-alert-details">
                    <span class="xc-alert-condition">{conditionLabel()}</span>
                    <span class="xc-alert-target-amount">
                      <MoneyText value={alert.target} currency="THB" />
                    </span>
                  </div>

                  <div class="xc-alert-meta">
                    <Show
                      when={alert.currentRate !== null && alert.ratePublishedAt !== null}
                      fallback={<span class="xc-alert-no-rate">{t('noCurrentRate')}</span>}
                    >
                      <span class="xc-alert-current-rate">
                        {t('currentRate')}: <MoneyText value={alert.currentRate!} currency="THB" />{' '}
                        <time datetime={alert.ratePublishedAt!}>
                          {fill(t('publishedAt'), { at: formatRateTime(alert.ratePublishedAt!) })}
                        </time>
                      </span>
                    </Show>
                  </div>

                  <div class="xc-alert-card-actions">
                    <Show
                      when={alert.enabled}
                      fallback={
                        <button
                          type="button"
                          class="xc-btn xc-btn-resume"
                          onClick={() => handleResume(alert)}
                        >
                          {t('resume')}
                        </button>
                      }
                    >
                      <button
                        type="button"
                        class="xc-btn xc-btn-pause"
                        onClick={() => handlePause(alert)}
                      >
                        {t('pause')}
                      </button>
                    </Show>

                    <button
                      type="button"
                      class="xc-btn xc-btn-delete"
                      onClick={() => handleDelete(alert)}
                    >
                      {t('delete')}
                    </button>
                  </div>
                </article>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
}
