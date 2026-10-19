// §9 Pricing: loads pricing_board, branch, filter (all · to approve · review · current), search, reload; composes the parts (FE pricing-board, #564; docs/design/blueprint/pricing-tiers.md §9).
import { Show, createEffect, createMemo, createSignal, onMount, type JSX } from 'solid-js';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { ApiClient } from '../../../shared/api/client';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import { BulkApprove } from './BulkApprove';
import * as table from './i18n';
import { MarketSources } from './MarketSources';
import { OrderRefs } from './OrderRefs';
import {
  FILTER_STATUSES,
  ROUTE_RATE_REFERENCE_FETCH,
  type RateReferenceFetchInput,
  VIEW_PRICING_BOARD,
  type MarketSourceCode,
  type PricingBoardData,
  type PricingFilter,
  type PricingRowView,
} from './pricing-board.types';
import { PricingGrid } from './PricingGrid';
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

function matchesSearch(row: PricingRowView, query: string): boolean {
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
  const [me, setMe] = createSignal<StaffMeView | null>(null);
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

  onMount(() => {
    void api
      .view<StaffMeView>('staff_me')
      .then((ans) => {
        if (ans.changed && ans.data !== undefined) setMe(ans.data);
      })
      .catch(() => {});
  });

  createEffect(() => {
    const branch = props.branch();
    void loadBoard(branch);
  });

  const filteredRows = createMemo<readonly PricingRowView[]>(() => {
    const rows = board()?.rows ?? [];
    const f = filter();
    const q = search();
    const allowed = f === 'all' ? null : FILTER_STATUSES[f];
    return rows.filter((r) => (!allowed || allowed.includes(r.status)) && matchesSearch(r, q));
  });

  const selectedRows = createMemo<readonly PricingRowView[]>(() => {
    const sel = selected();
    return (board()?.rows ?? []).filter((r) => sel.has(r.currency));
  });

  const openedRow = createMemo<PricingRowView | null>(() => {
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
    return Boolean(me()?.roles.some((r) => r.role === 'EXECUTIVE' && (r.branchId === null || r.branchId === branch)));
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

  const refreshWrite = keyedWrite<RateReferenceFetchInput, unknown>(api, 'POST', ROUTE_RATE_REFERENCE_FETCH);

  function handleRefreshSource(provider: MarketSourceCode): void {
    void refreshWrite
      .send({ branch: props.branch(), provider })
      .then(() => loadBoard())
      .catch((err) => setError(wordedError(err)));
  }

  return (
    <section class="pricing-screen">
      <div class="pricing-toolbar">
        <label class="pricing-filter-label">
          <span>{t('filter')}</span>
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
        <button class="pricing-reload" type="button" name="reload" onClick={() => void loadBoard()}>
          {t('reload')}
        </button>
      </div>

      <Show when={error()}>{(msg) => <p class="pricing-error">{msg()}</p>}</Show>
      <Show when={loading() && !board()}>
        <p class="pricing-loading">{t('loading')}</p>
      </Show>

      <MarketSources sources={board()?.sources ?? []} canRefresh={canRefresh()} onRefresh={handleRefreshSource} />

      <PricingGrid
        rows={filteredRows()}
        selected={selected()}
        onToggle={toggleRow}
        onOpen={(currency) => setOpenedCurrency(currency)}
      />

      <BulkApprove
        api={api}
        branch={props.branch()}
        policyVersionId={board()?.policyVersionId ?? ''}
        rows={selectedRows()}
        onApproved={() => void loadBoard()}
      />

      <Show when={openedRow()}>
        {(row) => (
          <div class="pricing-detail">
            <TierEditor
              api={api}
              branch={props.branch()}
              policyVersionId={board()?.policyVersionId ?? ''}
              row={row()}
              bands={board()?.bands ?? []}
              onApproved={() => void loadBoard()}
            />
            <PricingPolicy
              api={api}
              branch={props.branch()}
              policyVersionId={board()?.policyVersionId ?? ''}
              row={row()}
              mayEdit={mayEditPolicy()}
              onSaved={() => void loadBoard()}
            />
          </div>
        )}
      </Show>

      <div class="pricing-side">
        <Recommendations items={board()?.recommendations ?? []} />
        <OrderRefs refs={board()?.orderRefs ?? []} />
      </div>
    </section>
  );
}

