// §9 Pricing: loads pricing_board, branch, filter (all · to approve · review · current), search, reload; composes the parts (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type {
  MarketRatesRefreshInput,
  MarketRatesRefreshOutput,
} from '../../../shared/api/generated/market_rates_refresh';
import type { MeView } from '../../../shared/api/generated/me';
import type { PricingBoardData, PricingRowDto } from '../../../shared/api/generated/pricing_board';
import type {
  RateReferenceFetchInput,
  RateReferenceFetchOutput,
} from '../../../shared/api/generated/rate_reference_fetch';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { lang, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { ViewPoller } from '../../../shared/live/view-poller';
import { PermissionsUnavailable, permissionsUnavailableText } from '../../../shared/ui/PermissionsUnavailable';
import type { AdminMenuProps } from '../menus';
import { BranchDefault } from './BranchDefault';
import { BranchPolicy } from './BranchPolicy';
import { BulkApprove } from './BulkApprove';
import { DisplaySettings } from './DisplaySettings';
import * as table from './i18n';
import { MarketSources } from './MarketSources';
import {
  FILTER_STATUSES,
  ROUTE_PRICING_MARKET_REFRESH,
  ROUTE_RATE_REFERENCE_FETCH,
  VIEW_PRICING_BOARD,
  type MarketSourceCode,
  type PricingFilter,
  type RowStatusCode,
} from './pricing-labels';
import { currencyName } from './pricing-format';
import { PricingGrid } from './PricingGrid';
import { CardLines } from './PricingRow';
import { PricingMode } from './PricingMode';
import { PricingPolicy } from './PricingPolicy';
import { Recommendations } from './Recommendations';
import { TierEditor } from './TierEditor';
import './pricing.css';

export type PricingProps = AdminMenuProps;

const t = screenT('admin.pricing', table);
const enCurrencyNames =
  typeof Intl !== 'undefined' && typeof Intl.DisplayNames === 'function'
    ? new Intl.DisplayNames(['en'], { type: 'currency' })
    : null;

function matchesSearch(row: PricingRowDto, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  if (row.currency.toLowerCase().includes(q)) return true;
  if (enCurrencyNames) {
    try {
      const name = enCurrencyNames.of(row.currency);
      if (name && name.toLowerCase().includes(q)) return true;
    } catch {
      // Ignore invalid currency code
    }
  }
  return false;
}

export function Pricing(props: PricingProps): JSX.Element {
  const api = new ApiClient();
  const [board, setBoard] = createSignal<PricingBoardData | null>(null);
  const [me, setMe] = createSignal<MeView | null>(null);
  const [meError, setMeError] = createSignal<unknown>(null);
  const [meBusy, setMeBusy] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [filter, setFilter] = createSignal<PricingFilter>('all');
  const [search, setSearch] = createSignal('');
  const [selected, setSelected] = createSignal<Set<string>>(new Set());
  const [openedCurrency, setOpenedCurrency] = createSignal<string | null>(null);

  async function loadBoard(branch = props.branch()): Promise<void> {
    setLoading(true);
    setError(null);
    try {
      const ans = await api.view<PricingBoardData>(VIEW_PRICING_BOARD, { branch });
      if (ans.changed && ans.data !== undefined) {
        setBoard(ans.data);
      }
    } catch (err) {
      setError(wordedError(err));
    } finally {
      setLoading(false);
    }
  }

  async function loadMe(): Promise<void> {
    setMeBusy(true);
    try {
      const ans = await api.view<MeView>('me');
      if (ans.data !== undefined) setMe(ans.data);
      setMeError(null);
    } catch (err: unknown) {
      setMe(null);
      setMeError(err);
    } finally {
      setMeBusy(false);
    }
  }

  onMount(() => {
    void loadMe();
  });

  const paused = createMemo(() => openedCurrency() !== null || selected().size > 0);

  createEffect(() => {
    const branch = props.branch();
    const poller = new ViewPoller<PricingBoardData>(api, VIEW_PRICING_BOARD, {
      params: { branch },
      intervalMs: 60_000,
      maxIntervalMs: 300_000,
    });
    createEffect(() => {
      const data = poller.data();
      if (data !== undefined) setBoard(data);
    });
    createEffect(() => {
      if (paused()) {
        poller.stop();
      } else {
        poller.start();
      }
    });
    onCleanup(() => poller.stop());
  });

  const filteredRows = createMemo<readonly PricingRowDto[]>(() => {
    const rows = board()?.rows ?? [];
    const f = filter();
    const q = search();
    const allowed = f === 'all' ? null : FILTER_STATUSES[f];
    return rows.filter(
      (r) => (!allowed || allowed.includes(r.status as RowStatusCode)) && matchesSearch(r, q),
    );
  });

  const selectedRows = createMemo<readonly PricingRowDto[]>(() => {
    const sel = selected();
    return (board()?.rows ?? []).filter((r) => sel.has(r.currency));
  });

  const openedRow = createMemo<PricingRowDto | null>(() => {
    const cur = openedCurrency();
    if (!cur) return null;
    return (board()?.rows ?? []).find((r) => r.currency === cur) ?? null;
  });

  const canRefresh = createMemo(() => {
    const perms = me()?.permissions as readonly string[] | undefined;
    return Boolean(perms?.includes('MarketRatesRefresh'));
  });

  const mayEditPolicy = createMemo(() => {
    const branch = props.branch();
    return Boolean(
      me()?.roles.some((r) => r.role === 'EXECUTIVE' && (r.branchId === null || r.branchId === branch)),
    );
  });

  function toggleRow(currency: string): void {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(currency)) {
        next.delete(currency);
      } else {
        next.add(currency);
      }
      return next;
    });
  }

  const refreshWrite = keyedWrite<RateReferenceFetchInput, RateReferenceFetchOutput>(
    api,
    'POST',
    ROUTE_RATE_REFERENCE_FETCH,
  );
  const marketRefreshWrite = keyedWrite<MarketRatesRefreshInput, MarketRatesRefreshOutput>(
    api,
    'POST',
    ROUTE_PRICING_MARKET_REFRESH,
  );

  const disabledReason = () => (meError() !== null ? permissionsUnavailableText(meError()) : undefined);

  function handleRefreshSource(provider: MarketSourceCode): void {
    if (meError() !== null || !canRefresh()) return;
    if (provider === 'BOT') {
      void refreshWrite
        .send({})
        .then(() => loadBoard())
        .catch((err) => setError(wordedError(err)));
      return;
    }
    void marketRefreshWrite
      .send({})
      .then(async (out) => {
        await loadBoard();
        const failed = (out?.sources ?? []).filter((s) => !s.ok).map((s) => s.source);
        if (failed.length > 0) {
          setError(t('marketRefreshFailed', { source: failed.join(', ') }));
        }
      })
      .catch((err) => setError(wordedError(err)));
  }

  let closeRef: HTMLButtonElement | undefined;
  createEffect(() => {
    if (openedCurrency() !== null) queueMicrotask(() => closeRef?.focus());
  });

  return (
    <section class="pricing-screen" classList={{ 'pricing-screen-detail-open': openedRow() !== null }}>
      <div class="pricing-toolbar" role="search">
        <input
          class="pricing-search"
          type="search"
          name="search"
          placeholder={t('search')}
          aria-label={t('search')}
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
          onChange={(e) => setSearch(e.currentTarget.value)}
        />
        <label class="pricing-filter-label">
          <span class="pricing-filter-caption">{t('filter')}</span>
          <select
            class="pricing-filter"
            name="filter"
            aria-label={t('filter')}
            value={filter()}
            onChange={(e) => setFilter(e.currentTarget.value as PricingFilter)}
          >
            <option value="all">{t('filterAll')}</option>
            <option value="toApprove">{t('filterToApprove')}</option>
            <option value="review">{t('filterReview')}</option>
            <option value="current">{t('filterCurrent')}</option>
          </select>
        </label>
        <button class="pricing-reload" type="button" name="reload" onClick={() => void loadBoard()}>
          {t('reload')}
        </button>
      </div>

      <PermissionsUnavailable error={meError()} onRetry={() => void loadMe()} busy={meBusy()} />
      <Show when={error()}>{(msg) => <p class="pricing-error" role="alert">{msg()}</p>}</Show>
      <Show when={loading() && !board()}>
        <p class="pricing-loading">{t('loading')}</p>
      </Show>

      <section class="pricing-panel pricing-sources" aria-label={t('sourcesTitle')}>
        <h3 class="pricing-panel-title">{t('sourcesTitle')}</h3>
        <MarketSources
          sources={board()?.sources ?? []}
          canRefresh={canRefresh()}
          disabledReason={disabledReason()}
          onRefresh={handleRefreshSource}
        />
      </section>

      <PricingGrid
        rows={filteredRows()}
        selected={selected()}
        onToggle={toggleRow}
        onOpen={(currency) => setOpenedCurrency(currency)}
        recommendations={board()?.recommendations ?? []}
        opened={openedCurrency()}
      />

      <div class="pricing-bulk-bar" classList={{ 'pricing-bulk-bar-active': selected().size > 0 }}>
        <BulkApprove
          api={api}
          branch={props.branch()}
          policyVersionId={board()?.policyVersionId ?? ''}
          rows={selectedRows()}
          onApproved={() => void loadBoard()}
        />
      </div>

      <div class="pricing-side">
        <Recommendations items={board()?.recommendations ?? []} />
        {/* hidden until the order-reference view exists (design §7) */}
      </div>

      <Show when={openedRow()}>
        {(row) => (
          <>
            <div class="pricing-detail-backdrop" aria-hidden="true" onClick={() => setOpenedCurrency(null)} />
            <div
              class="pricing-detail"
              role="dialog"
              aria-modal="true"
              aria-labelledby="pricing-detail-title"
              onKeyDown={(e) => {
                if (e.key === 'Escape') setOpenedCurrency(null);
              }}
            >
              <header class="pricing-detail-head">
                <div class="pricing-card-title">
                  <h3 class="pricing-detail-code" id="pricing-detail-title">
                    {row().currency}
                  </h3>
                  <Show when={currencyName(row().currency, lang())}>
                    {(n) => <span class="pricing-card-name">{n()}</span>}
                  </Show>
                </div>
                <button
                  ref={(el) => (closeRef = el)}
                  type="button"
                  name="close-detail"
                  class="pricing-btn-outline"
                  onClick={() => setOpenedCurrency(null)}
                >
                  {t('closeDetail')}
                </button>
              </header>
              <div class="pricing-detail-summary">
                <CardLines row={row()} />
              </div>
              <div class="pricing-detail-body">
                <TierEditor
                  api={api}
                  branch={props.branch()}
                  policyVersionId={board()?.policyVersionId ?? ''}
                  row={row()}
                  bands={board()?.bands ?? []}
                  onApproved={() => void loadBoard()}
                />
                <div class="pricing-detail-section">
                  <PricingPolicy
                    api={api}
                    branch={props.branch()}
                    policyVersionId={board()?.policyVersionId ?? ''}
                    row={row()}
                    mayEdit={mayEditPolicy()}
                    disabledReason={disabledReason()}
                    onSaved={() => void loadBoard()}
                  />
                </div>
                <div class="pricing-detail-section">
                  <PricingMode
                    api={api}
                    branch={props.branch()}
                    row={row()}
                    isMain={board()?.isMain ?? false}
                    mayEdit={mayEditPolicy()}
                    disabledReason={disabledReason()}
                    onSaved={() => void loadBoard()}
                  />
                </div>
                <div class="pricing-detail-section">
                  <BranchPolicy
                    api={api}
                    branch={props.branch()}
                    row={row()}
                    isMain={board()?.isMain ?? false}
                    mayEdit={mayEditPolicy()}
                    disabledReason={disabledReason()}
                    onSaved={() => void loadBoard()}
                  />
                </div>
              </div>
            </div>
          </>
        )}
      </Show>

      <Show when={board()}>
        {(b) => (
          <section class="pricing-settings" aria-label={t('settingsTitle')}>
            <h3 class="pricing-panel-title">{t('settingsTitle')}</h3>
            <div class="pricing-settings-grid">
              <div class="pricing-panel">
                <BranchDefault
                  api={api}
                  branch={props.branch()}
                  value={b().branchDefault}
                  mayEdit={mayEditPolicy()}
                  disabledReason={disabledReason()}
                  onSaved={() => void loadBoard()}
                />
              </div>
              <div class="pricing-panel">
                <DisplaySettings
                  api={api}
                  branch={props.branch()}
                  value={b().display}
                  currencies={b().rows.map((r) => r.currency)}
                  mayEdit={mayEditPolicy()}
                  disabledReason={disabledReason()}
                  onSaved={() => void loadBoard()}
                />
              </div>
            </div>
          </section>
        )}
      </Show>
    </section>
  );
}
