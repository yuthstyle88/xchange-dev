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
import { keyedWrite } from '../../../shared/api/keyed-write';
import { readHostOrg } from '../../../shared/api/public-host';
import type {
  CreateRateAlertInput,
  MyRateAlertItem,
  MyRateAlertsData,
} from '../../../shared/api/generated/my_rate_alerts';
import type {
  PublicBranchDto,
  PublicBranchesData,
} from '../../../shared/api/public/public_branches';
import { fill, locale, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { MoneyText } from '../../../shared/ui/MoneyText';
import * as table from './i18n';
import { AlertFacts } from './AlertFacts';
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

export function RateAlertsScreen(props: RateAlertsScreenProps): JSX.Element {
  const fetchFn = props.fetch ?? ((input, init) => fetch(input, init));
  const api = props.client ?? new ApiClient(fetchFn);

  const createApi = {
    command: async <B, R>(
      method: string,
      path: string,
      body: B,
      key: string,
    ): Promise<R> => {
      try {
        return await api.command<B, R>(method as 'POST', path, body, key);
      } catch (err: unknown) {
        if (err instanceof ApiError && err.code === 'alertLimitReached') {
          throw { code: err.code, message: err.message, status: err.status };
        }
        throw err;
      }
    },
  } as unknown as ApiClient;
  const createWrite = keyedWrite<CreateRateAlertInput, unknown>(
    createApi,
    'POST',
    '/api/app/rate-alerts',
  );
  const versionedApi = {
    command: (method: string, path: string, body: { id: string; expectedVersion: number }, key: string) =>
      api.command(method, path, { expectedVersion: body.expectedVersion }, key),
  } as unknown as ApiClient;
  const pauseWrite = keyedWrite<{ id: string; expectedVersion: number }, unknown>(
    versionedApi,
    'POST',
    (b) => `/api/app/rate-alerts/${encodeURIComponent(b.id)}/pause`,
  );
  const resumeWrite = keyedWrite<{ id: string; expectedVersion: number }, unknown>(
    versionedApi,
    'POST',
    (b) => `/api/app/rate-alerts/${encodeURIComponent(b.id)}/resume`,
  );
  const deleteWrite = keyedWrite<{ id: string; expectedVersion: number }, unknown>(
    versionedApi,
    'POST',
    (b) => `/api/app/rate-alerts/${encodeURIComponent(b.id)}/delete`,
  );

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
  const [isCreating, setIsCreating] = createSignal(false);
  const [createError, setCreateError] = createSignal<string | null>(null);

  // Action status/error
  const [actionError, setActionError] = createSignal<string | null>(null);
  const [inFlightAlertIds, setInFlightAlertIds] = createSignal<Set<string>>(new Set());

  const isAlertInFlight = (id: string) => inFlightAlertIds().has(id);
  const addInFlightAlert = (id: string) => {
    setInFlightAlertIds((prev) => {
      const next = new Set(prev);
      next.add(id);
      return next;
    });
  };
  const removeInFlightAlert = (id: string) => {
    setInFlightAlertIds((prev) => {
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
  };

  let sinceVersion: number | undefined = undefined;
  let timerId: ReturnType<typeof setInterval> | undefined = undefined;
  let loadSeq = 0;

  const ALERT_ERROR_KEYS: Record<string, keyof typeof table.th> = {
    alertLimitReached: 'alertLimitReached',
    alertSettingsMissing: 'alertSettingsMissing',
    idempotencyConflict: 'idempotencyConflict',
    staleVersion: 'staleVersion',
    validationFailed: 'validationFailed',
  };

  const rateAlertsError = (err: unknown): string => {
    if (err instanceof ApiError) {
      const mapped = ALERT_ERROR_KEYS[err.code];
      if (mapped) {
        const text = t(mapped);
        if (text && text !== mapped) return text;
      }
    } else if (
      err &&
      typeof err === 'object' &&
      'code' in err &&
      typeof (err as { code: unknown }).code === 'string'
    ) {
      const mapped = ALERT_ERROR_KEYS[(err as { code: string }).code];
      if (mapped) {
        const text = t(mapped);
        if (text && text !== mapped) return text;
      }
    }
    return wordedError(err);
  };

  const loadBranches = async () => {
    try {
      const org = props.org || (await readHostOrg(api));
      const bJson = await api.publicGet<PublicBranchesData>(
        `/api/public/orgs/${encodeURIComponent(org)}/branches`,
      );
      const list = bJson.branches ?? [];
      setBranches(list);
      setBranchesError(null);
      if (list.length > 0 && !branch()) {
        setBranch(list[0].code);
      }
    } catch (err: unknown) {
      setBranchesError(wordedError(err));
    }
  };

  const loadAlerts = async (forceFull = false) => {
    const seq = ++loadSeq;
    if (forceFull) {
      sinceVersion = undefined;
    }
    if (alerts().length === 0) {
      setLoading(true);
    }
    setLoadError(null);
    try {
      const answer = await api.view<MyRateAlertsData>('my_rate_alerts', { since: sinceVersion });
      if (seq !== loadSeq) return;
      if (answer.changed && answer.data) {
        setAlerts(answer.data.alerts ?? []);
        sinceVersion = answer.version;
      } else if (sinceVersion === undefined && answer.data) {
        setAlerts(answer.data.alerts ?? []);
        sinceVersion = answer.version;
      }
      // If changed === false, keep alerts as is per §8
    } catch (err: unknown) {
      if (seq !== loadSeq) return;
      if (alerts().length === 0) {
        setLoadError(rateAlertsError(err));
      }
    } finally {
      if (seq === loadSeq) {
        setLoading(false);
      }
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
      await createWrite.send(input);
      // On success: clear form inputs and refresh
      setTarget('');
      setExpiresAt('');
      await loadAlerts(true);
    } catch (err: unknown) {
      // On error (including alertLimitReached): keep form inputs
      setCreateError(rateAlertsError(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        void loadAlerts(true);
      }
    } finally {
      setIsCreating(false);
    }
  };

  const handlePause = async (alert: MyRateAlertItem) => {
    if (isAlertInFlight(alert.id)) return;
    addInFlightAlert(alert.id);
    setActionError(null);
    try {
      await pauseWrite.send({ id: alert.id, expectedVersion: alert.version });
      await loadAlerts(true);
    } catch (err: unknown) {
      setActionError(rateAlertsError(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        await loadAlerts(true);
      }
    } finally {
      removeInFlightAlert(alert.id);
    }
  };

  const handleResume = async (alert: MyRateAlertItem) => {
    if (isAlertInFlight(alert.id)) return;
    addInFlightAlert(alert.id);
    setActionError(null);
    try {
      await resumeWrite.send({ id: alert.id, expectedVersion: alert.version });
      await loadAlerts(true);
    } catch (err: unknown) {
      setActionError(rateAlertsError(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        await loadAlerts(true);
      }
    } finally {
      removeInFlightAlert(alert.id);
    }
  };

  const handleDelete = async (alert: MyRateAlertItem) => {
    if (isAlertInFlight(alert.id)) return;
    addInFlightAlert(alert.id);
    setActionError(null);
    try {
      await deleteWrite.send({ id: alert.id, expectedVersion: alert.version });
      await loadAlerts(true);
    } catch (err: unknown) {
      setActionError(rateAlertsError(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        await loadAlerts(true);
      }
    } finally {
      removeInFlightAlert(alert.id);
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

                  <AlertFacts alert={alert} />

                  <div class="xc-alert-card-actions">
                    <Show
                      when={alert.enabled}
                      fallback={
                        <button
                          type="button"
                          class="xc-btn xc-btn-resume"
                          disabled={isAlertInFlight(alert.id)}
                          onClick={() => handleResume(alert)}
                        >
                          {t('resume')}
                        </button>
                      }
                    >
                      <button
                        type="button"
                        class="xc-btn xc-btn-pause"
                        disabled={isAlertInFlight(alert.id)}
                        onClick={() => handlePause(alert)}
                      >
                        {t('pause')}
                      </button>
                    </Show>

                    <button
                      type="button"
                      class="xc-btn xc-btn-delete"
                      disabled={isAlertInFlight(alert.id)}
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
