// APP-8 (C01 customer home screen, `/app`).
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  untrack,
  type JSX,
} from 'solid-js';
import { fill, lang, screenT } from '../../../shared/i18n';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { PublicPoller } from '../../../shared/live/public-poller';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { PublicBranchDto, PublicBranchesData } from '../../../shared/api/public/public_branches';
import type { PlannerPreviewAnswer } from '../../../shared/api/generated/planner_preview';
import type { PlannerDraftDto } from '../../../shared/api/generated/my_planner_drafts';
import { MoneyText } from '../../../shared/ui/MoneyText';
import { RateFreshness } from '../../../shared/ui/RateFreshness';
import { formatOpeningHours } from '../../../shared/ui/openingHours';
import { formatRate } from '../../../shared/ui/rate';
import { th, en, zh } from './i18n';
import './home.css';

export interface HomeScreenProps {
  /** Injected fetch for testing. Defaults to window.fetch. */
  fetch?: (url: string, init?: RequestInit) => Promise<Response>;
  /** Optional org code. If not provided, fetched from /api/public/host. */
  org?: string;
  /** Optional preselected branch, or search string like `?branch=XC-B01`. */
  search?: string;
  /** Polling interval for rates in ms. */
  ratesIntervalMs?: number;
  /** Polling interval for branches in ms. */
  branchesIntervalMs?: number;
}

export function branchFromSearch(search?: string): string | null {
  if (!search) return null;
  const params = new URLSearchParams(search);
  const branch = params.get('branch');
  return branch && branch.trim().length > 0 ? branch.trim() : null;
}

export function HomeScreen(props: HomeScreenProps): JSX.Element {
  const t = screenT('customer.home', { th, en, zh });
  const doFetch = props.fetch ?? ((url, init) => fetch(url, init));
  const api = new ApiClient(doFetch);

  const [org, setOrg] = createSignal<string | null>(props.org ?? null);
  const [orgState, setOrgState] = createSignal<'loading' | 'ready' | 'error'>(
    props.org ? 'ready' : 'loading',
  );

  const [branchesPoller, setBranchesPoller] = createSignal<
    PublicPoller<PublicBranchesData> | undefined
  >(undefined);
  const [ratesPoller, setRatesPoller] = createSignal<
    PublicPoller<PublishedRatesData> | undefined
  >(undefined);

  const [selectedBranchCode, setSelectedBranchCode] = createSignal<string>(
    branchFromSearch(props.search) ?? '',
  );

  // Planner signals
  const [calcCurrency, setCalcCurrency] = createSignal<string>('USD');
  const [customerSide, setCustomerSide] = createSignal<'CUSTOMER_BUY' | 'CUSTOMER_SELL'>('CUSTOMER_BUY');
  const [amountType, setAmountType] = createSignal<'quantity' | 'budgetThb'>('quantity');
  const [amount, setAmount] = createSignal<string>('100');
  const [previewLoading, setPreviewLoading] = createSignal<boolean>(false);
  const [previewError, setPreviewError] = createSignal<string | null>(null);
  const [previewResult, setPreviewResult] = createSignal<PlannerPreviewAnswer | null>(null);

  const [savingPlan, setSavingPlan] = createSignal<boolean>(false);
  const [savedPlanId, setSavedPlanId] = createSignal<string | null>(null);
  const [savePlanError, setSavePlanError] = createSignal<string | null>(null);

  // Initialize Org
  onMount(async () => {
    if (props.org) {
      setOrg(props.org);
      setOrgState('ready');
      return;
    }
    try {
      const res = await doFetch('/api/public/host');
      if (res.status === 200) {
        const json = (await res.json()) as { org?: string };
        if (json && typeof json.org === 'string' && json.org.length > 0) {
          setOrg(json.org);
          setOrgState('ready');
          return;
        }
      }
      setOrgState('error');
    } catch {
      setOrgState('error');
    }
  });

  // Setup Branches Poller when Org is ready
  createEffect(() => {
    const currentOrg = org();
    if (orgState() !== 'ready' || !currentOrg) {
      setBranchesPoller(undefined);
      return;
    }

    const branchesUrl = `/api/public/orgs/${encodeURIComponent(currentOrg)}/branches`;
    const bPoller = new PublicPoller<PublicBranchesData>(
      branchesUrl,
      doFetch,
      props.branchesIntervalMs ?? 600_000,
      600_000,
    );
    setBranchesPoller(bPoller);
    bPoller.start();

    onCleanup(() => {
      bPoller.stop();
    });
  });

  // Keep selected branch updated when branches load
  createEffect(() => {
    const bData = branchesPoller()?.data();
    const branches = bData?.branches;
    if (branches && branches.length > 0) {
      const current = untrack(() => selectedBranchCode());
      const match = branches.find((b) => b.code === current);
      if (!match) {
        const fromSearch = branchFromSearch(props.search);
        const searchMatch = fromSearch ? branches.find((b) => b.code === fromSearch) : undefined;
        setSelectedBranchCode(searchMatch ? searchMatch.code : branches[0]!.code);
      }
    }
  });

  // Setup Rates Poller when Branch or Org changes
  createEffect(() => {
    const currentOrg = org();
    const branchCode = selectedBranchCode();

    if (orgState() !== 'ready' || !currentOrg || !branchCode) {
      setRatesPoller(undefined);
      return;
    }

    const ratesUrl = `/api/public/orgs/${encodeURIComponent(currentOrg)}/branches/${encodeURIComponent(branchCode)}/rates`;
    const rPoller = new PublicPoller<PublishedRatesData>(
      ratesUrl,
      doFetch,
      props.ratesIntervalMs ?? 60_000,
      300_000,
    );
    setRatesPoller(rPoller);
    rPoller.start();

    onCleanup(() => {
      rPoller.stop();
    });
  });

  onCleanup(() => {
    branchesPoller()?.stop();
    ratesPoller()?.stop();
  });

  const branches = () => branchesPoller()?.data()?.branches ?? [];
  const branchesState = () => {
    if (orgState() === 'loading') return 'loading';
    if (orgState() === 'error') return 'error';
    const state = branchesPoller()?.state() ?? 'loading';
    if (state === 'ready' && branches().length === 0) return 'empty';
    return state;
  };

  const selectedBranch = createMemo<PublicBranchDto | undefined>(() => {
    const code = selectedBranchCode();
    return branches().find((b) => b.code === code);
  });

  const ratesData = () => ratesPoller()?.data();
  const ratesLines = () => ratesData()?.lines ?? [];
  const ratesState = () => {
    if (branchesState() === 'loading' || !selectedBranchCode()) return 'loading';
    if (branchesState() === 'error') return 'error';
    if (branchesState() === 'empty') return 'empty';
    const state = ratesPoller()?.state() ?? 'loading';
    if (state === 'ready' && ratesLines().length === 0) return 'empty';
    return state;
  };

  const availableCurrencies = createMemo(() => {
    const lines = ratesLines();
    if (lines.length === 0) return ['USD', 'EUR', 'JPY', 'GBP', 'CNY', 'SGD', 'KRW', 'AUD', 'HKD'];
    return Array.from(new Set(lines.map((l) => l.currency)));
  });

  const handleRetryBranches = () => {
    if (orgState() === 'error' && !props.org) {
      setOrgState('loading');
      doFetch('/api/public/host')
        .then(async (res) => {
          if (res.status === 200) {
            const json = (await res.json()) as { org?: string };
            if (json && typeof json.org === 'string' && json.org.length > 0) {
              setOrg(json.org);
              setOrgState('ready');
              return;
            }
          }
          setOrgState('error');
        })
        .catch(() => setOrgState('error'));
      return;
    }
    const bPoller = branchesPoller();
    if (bPoller) {
      bPoller.stop();
      bPoller.start();
    }
  };

  const handleRetryRates = () => {
    const rPoller = ratesPoller();
    if (rPoller) {
      rPoller.stop();
      rPoller.start();
    }
  };

  let pendingSave: { key: string; bodyJson: string } | null = null;

  const handleCalculate = async () => {
    const amtStr = amount().trim();
    const isNum = /^\d+(\.\d+)?$/.test(amtStr) && parseFloat(amtStr) > 0;
    if (!isNum) {
      setPreviewError(t('validationError'));
      setPreviewResult(null);
      return;
    }

    const branch = selectedBranchCode();
    if (!branch) {
      setPreviewError(t('validationError'));
      setPreviewResult(null);
      return;
    }

    setPreviewLoading(true);
    setPreviewError(null);
    setPreviewResult(null);

    const body: Record<string, unknown> = {
      branch,
      currency: calcCurrency(),
      customerSide: customerSide(),
      amount: amountType() === 'quantity' ? { quantity: amtStr } : { budgetThb: amtStr },
    };

    try {
      const json = await api.act<Record<string, unknown>, PlannerPreviewAnswer>(
        'compute',
        'planner_preview',
        body,
      );
      setPreviewResult(json);
    } catch (err) {
      if (err instanceof ApiError && !isServerFailure(err) && err.code === 'rateUnavailable') {
        setPreviewError(t('rateUnavailable'));
      } else if (
        err instanceof ApiError &&
        !isServerFailure(err) &&
        err.code === 'validationFailed'
      ) {
        setPreviewError(t('validationError'));
      } else {
        setPreviewError(wordedError(err));
      }
    } finally {
      setPreviewLoading(false);
    }
  };

  const handleSavePlan = async () => {
    const amtStr = amount().trim();
    const isNum = /^\d+(\.\d+)?$/.test(amtStr) && parseFloat(amtStr) > 0;
    if (!isNum) {
      setSavePlanError(t('validationError'));
      return;
    }

    const branch = selectedBranchCode();
    setSavingPlan(true);
    setSavedPlanId(null);
    setSavePlanError(null);

    const body: Record<string, unknown> = {
      branch: branch || undefined,
      currency: calcCurrency(),
      customerSide: customerSide(),
      amount: amountType() === 'quantity' ? { quantity: amtStr } : { budgetThb: amtStr },
    };
    const bodyJson = JSON.stringify(body);
    if (!pendingSave || pendingSave.bodyJson !== bodyJson) {
      const nextKey =
        typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
          ? crypto.randomUUID()
          : `plan-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      pendingSave = { key: nextKey, bodyJson };
    }
    const idempotencyKey = pendingSave.key;

    try {
      const draft = await api.command<Record<string, unknown>, PlannerDraftDto>(
        'POST',
        '/api/app/planner-drafts',
        body,
        idempotencyKey,
      );
      pendingSave = null;
      setSavedPlanId(draft.id);
    } catch (err) {
      setSavePlanError(wordedError(err));
    } finally {
      setSavingPlan(false);
    }
  };

  return (
    <div class="xc-customer-home">
      {/* Top Header */}
      <header class="xc-home-header">
        <div class="xc-home-header-brand">
          <h1 class="xc-home-title">XCHANGE</h1>
          <p class="xc-home-tagline">{t('welcome')}</p>
        </div>
      </header>

      {/* Main Content */}
      <main class="xc-home-main">
        {/* Branch Selection Section */}
        <section class="xc-home-card xc-home-branch-section" aria-labelledby="branch-heading">
          <h2 id="branch-heading" class="xc-home-card-title">{t('selectBranch')}</h2>

          <Show when={branchesState() === 'loading'}>
            <div class="xc-home-loading" role="status">
              <span>{t('loadingBranches')}</span>
            </div>
          </Show>

          <Show when={branchesState() === 'error'}>
            <div class="xc-home-error" role="alert">
              <p>{t('loadBranchesFailed')}</p>
              <button
                type="button"
                class="xc-home-retry-btn"
                onClick={handleRetryBranches}
              >
                {t('retry')}
              </button>
            </div>
          </Show>

          <Show when={branchesState() === 'empty'}>
            <div class="xc-home-empty">{t('emptyBranches')}</div>
          </Show>

          <Show when={branchesState() === 'ready'}>
            <div class="xc-home-branch-picker">
              <label for="branch-select" class="xc-home-label">
                {t('branch')}
              </label>
              <select
                id="branch-select"
                class="xc-home-select"
                value={selectedBranchCode()}
                onChange={(e) => setSelectedBranchCode(e.currentTarget.value)}
              >
                <For each={branches()}>
                  {(b) => <option value={b.code}>{b.name} ({b.code})</option>}
                </For>
              </select>
            </div>

            <Show when={selectedBranch()}>
              {(b) => (
                <div class="xc-home-branch-detail">
                  <div class="xc-home-branch-name">{b().name}</div>
                  <Show when={b().address}>
                    <p class="xc-home-branch-address">{b().address}</p>
                  </Show>
                  <Show when={formatOpeningHours(b().openingHours, lang())}>
                    {(hours) => (
                      <p class="xc-home-branch-hours">
                        <strong>{t('hours')}:</strong> {hours()}
                      </p>
                    )}
                  </Show>
                  <div class="xc-home-branch-badges">
                    <span
                      class={`xc-home-badge ${b().bookingEnabled ? 'xc-home-badge-success' : 'xc-home-badge-muted'}`}
                    >
                      {b().bookingEnabled ? t('bookingEnabled') : t('bookingDisabled')}
                    </span>
                    <span
                      class={`xc-home-badge ${b().queueOpen ? 'xc-home-badge-success' : 'xc-home-badge-muted'}`}
                    >
                      {b().queueOpen ? t('queueOpen') : t('queueClosed')}
                    </span>
                  </div>
                </div>
              )}
            </Show>
          </Show>
        </section>

        {/* Currency Planner / Calculator */}
        <section class="xc-home-card xc-home-planner-section" aria-labelledby="calc-heading">
          <h2 id="calc-heading" class="xc-home-card-title">{t('calculatorTitle')}</h2>

          <div class="xc-home-form-group">
            <span class="xc-home-label">{t('customerSide')}</span>
            <div class="xc-home-toggle-group">
              <button
                type="button"
                class={`xc-home-toggle-btn ${customerSide() === 'CUSTOMER_BUY' ? 'active' : ''}`}
                onClick={() => setCustomerSide('CUSTOMER_BUY')}
                aria-pressed={customerSide() === 'CUSTOMER_BUY'}
              >
                {t('customerBuy')}
              </button>
              <button
                type="button"
                class={`xc-home-toggle-btn ${customerSide() === 'CUSTOMER_SELL' ? 'active' : ''}`}
                onClick={() => setCustomerSide('CUSTOMER_SELL')}
                aria-pressed={customerSide() === 'CUSTOMER_SELL'}
              >
                {t('customerSell')}
              </button>
            </div>
          </div>

          <div class="xc-home-form-row">
            <div class="xc-home-form-group xc-home-flex-1">
              <label for="currency-select" class="xc-home-label">{t('currency')}</label>
              <select
                id="currency-select"
                class="xc-home-select"
                value={calcCurrency()}
                onChange={(e) => setCalcCurrency(e.currentTarget.value)}
              >
                <For each={availableCurrencies()}>
                  {(c) => <option value={c}>{c}</option>}
                </For>
              </select>
            </div>

            <div class="xc-home-form-group xc-home-flex-1">
              <label for="amount-type-select" class="xc-home-label">{t('amountType')}</label>
              <select
                id="amount-type-select"
                class="xc-home-select"
                value={amountType()}
                onChange={(e) => setAmountType(e.currentTarget.value as 'quantity' | 'budgetThb')}
              >
                <option value="quantity">{t('quantity')}</option>
                <option value="budgetThb">{t('budgetThb')}</option>
              </select>
            </div>
          </div>

          <div class="xc-home-form-group">
            <label for="amount-input" class="xc-home-label">{t('amount')}</label>
            <div class="xc-home-input-wrapper">
              <input
                id="amount-input"
                type="text"
                inputmode="decimal"
                class="xc-home-input"
                value={amount()}
                onInput={(e) => setAmount(e.currentTarget.value)}
              />
              <span class="xc-home-input-suffix">
                {amountType() === 'quantity' ? calcCurrency() : 'THB'}
              </span>
            </div>
          </div>

          <div class="xc-home-calc-actions">
            <button
              type="button"
              class="xc-home-btn xc-home-btn-primary"
              onClick={handleCalculate}
              disabled={previewLoading()}
            >
              {previewLoading() ? t('calculating') : t('calculate')}
            </button>
          </div>

          <Show when={previewError()}>
            {(err) => (
              <div class="xc-home-alert xc-home-alert-error" role="alert">
                {err()}
              </div>
            )}
          </Show>

          {/* Planner Estimate Result */}
          <Show when={previewResult()}>
            {(result) => (
              <div class="xc-home-estimate-card" role="region" aria-label={t('estimateResult')}>
                <h3 class="xc-home-estimate-title">{t('estimateResult')}</h3>

                <div class="xc-home-estimate-row">
                  <span class="xc-home-estimate-label">{t('estimatedThb')}:</span>
                  <span class="xc-home-estimate-value">
                    <MoneyText value={result().thb} currency="THB" />
                  </span>
                </div>

                <div class="xc-home-estimate-meta">
                  <span>{t('appliedRate')}: <strong>{result().rate}</strong></span>
                  <span> · {t('unit')}: <strong>{result().unitSize} {calcCurrency()}</strong></span>
                </div>

                <RateFreshness
                  stale={result().stale}
                  sourceDate={result().sourceDate}
                  publishedAt={null}
                />

                {/* P5: Disclaimer that an estimate is never a final rate */}
                <div class="xc-home-disclaimer" role="note">
                  <span class="xc-home-disclaimer-icon">ℹ️</span>
                  <span>{t('disclaimer')}</span>
                </div>

                <div class="xc-home-save-actions">
                  <button
                    type="button"
                    class="xc-home-btn xc-home-btn-secondary"
                    onClick={handleSavePlan}
                    disabled={savingPlan()}
                  >
                    {savingPlan() ? t('savingPlan') : t('savePlan')}
                  </button>
                </div>

                <Show when={savedPlanId()}>
                  {(id) => (
                    <div class="xc-home-alert xc-home-alert-success" role="status">
                      {fill(t('planSaved'), { id: id() })}
                    </div>
                  )}
                </Show>

                <Show when={savePlanError()}>
                  {(err) => (
                    <div class="xc-home-alert xc-home-alert-error" role="alert">
                      {err()}
                    </div>
                  )}
                </Show>
              </div>
            )}
          </Show>
        </section>

        {/* Exchange Rate Board Section */}
        <section class="xc-home-card xc-home-rates-section" aria-labelledby="rates-heading">
          <div class="xc-home-rates-header">
            <h2 id="rates-heading" class="xc-home-card-title">{t('ratesTitle')}</h2>
          </div>

          <Show when={ratesState() === 'loading'}>
            <div class="xc-home-loading" role="status">
              <span>{t('loadingRates')}</span>
            </div>
          </Show>

          <Show when={ratesState() === 'error'}>
            <div class="xc-home-error" role="alert">
              <p>{t('loadRatesFailed')}</p>
              <button
                type="button"
                class="xc-home-retry-btn"
                onClick={handleRetryRates}
              >
                {t('retry')}
              </button>
            </div>
          </Show>

          <Show when={ratesState() === 'empty'}>
            <div class="xc-home-empty">{t('emptyRates')}</div>
          </Show>

          <Show when={ratesState() === 'ready'}>
            <div class="xc-home-table-wrapper">
              <table class="xc-home-table">
                <thead>
                  <tr>
                    <th>{t('currency')}</th>
                    <th>{t('unit')}</th>
                    <th>{t('weBuy')}</th>
                    <th>{t('weSell')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={ratesLines()}>
                    {(line) => (
                      <tr>
                        <td class="xc-home-cell-currency">
                          <strong>{line.currency}</strong>
                          <Show when={line.group && line.group !== 'ALL'}>
                            <span class="xc-home-cell-group">({line.group})</span>
                          </Show>
                        </td>
                        <td>{line.unitSize} {line.currency}</td>
                        <td class="xc-home-cell-rate">
                          {line.buyRate !== null ? formatRate(line.buyRate, '1', line.currency) : '—'}
                        </td>
                        <td class="xc-home-cell-rate">
                          {line.sellRate !== null ? formatRate(line.sellRate, '1', line.currency) : '—'}
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>

            <RateFreshness
              stale={ratesData()?.stale ?? false}
              sourceDate={ratesData()?.sourceDate}
              publishedAt={ratesData()?.publication?.publishedAt}
            />
          </Show>
        </section>

        {/* Quick Service Links */}
        <section class="xc-home-card xc-home-services-section" aria-labelledby="services-heading">
          <h2 id="services-heading" class="xc-home-card-title">{t('quickServices')}</h2>
          <div class="xc-home-services-grid">
            <a href="/app/book" class="xc-home-service-link">
              <span class="xc-home-service-icon">📅</span>
              <span class="xc-home-service-name">{t('bookService')}</span>
            </a>
            <a href="/app/branches" class="xc-home-service-link">
              <span class="xc-home-service-icon">📍</span>
              <span class="xc-home-service-name">{t('branchesService')}</span>
            </a>
            <a href="/app/journey" class="xc-home-service-link">
              <span class="xc-home-service-icon">🎫</span>
              <span class="xc-home-service-name">{t('journeyService')}</span>
            </a>
            <a href="/app/history" class="xc-home-service-link">
              <span class="xc-home-service-icon">📜</span>
              <span class="xc-home-service-name">{t('historyService')}</span>
            </a>
            <a href="/app/assistant" class="xc-home-service-link">
              <span class="xc-home-service-icon">💬</span>
              <span class="xc-home-service-name">{t('assistantService')}</span>
            </a>
            <a href="/app/notifications" class="xc-home-service-link">
              <span class="xc-home-service-icon">🔔</span>
              <span class="xc-home-service-name">{t('notificationsService')}</span>
            </a>
          </div>
        </section>
      </main>
    </div>
  );
}
