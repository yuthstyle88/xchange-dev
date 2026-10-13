// C09 customer journey screen (design customer-app.md §4.5, §9.1)
import {
  createSignal,
  For,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import type {
  MyPlannerDraftsData,
  PlannerDraftDto,
} from '../../../shared/api/generated/my_planner_drafts';
import type { MyQueueTicketData } from '../../../shared/api/generated/my_queue_ticket';
import { locale, screenT } from '../../../shared/i18n';
import { MoneyText } from '../../../shared/ui/MoneyText';
import { en, th, zh } from './i18n';
import './journey.css';

export interface JourneyScreenProps {
  /** Injected so tests mock at the fetch level; defaults to browser's `fetch`. */
  fetch?: (url: string, init?: RequestInit) => Promise<Response>;
  /** `window.location.search`, e.g. `?ticketId=...&branch=...`. */
  search?: string;
  ticketId?: string;
  branch?: string;
}

type State = 'loading' | 'ready' | 'empty' | 'error' | 'unauthorized';

export function JourneyScreen(props: JourneyScreenProps): JSX.Element {
  const t = screenT('customer.journey', { th, en, zh });
  const fetchFn = (url: string, init?: RequestInit) => {
    if (props.fetch) return props.fetch(url, init);
    return fetch(url, init);
  };

  const [queueState, setQueueState] = createSignal<State>('loading');
  const [ticket, setTicket] = createSignal<MyQueueTicketData | null>(null);

  const [draftsState, setDraftsState] = createSignal<State>('loading');
  const [drafts, setDrafts] = createSignal<PlannerDraftDto[]>([]);

  const searchParams = () => {
    const s = props.search ?? (typeof window !== 'undefined' ? window.location.search : '');
    return new URLSearchParams(s);
  };

  const activeTicketId = () => {
    if (props.ticketId) return props.ticketId;
    const fromSearch = searchParams().get('ticketId') ?? searchParams().get('ticket');
    if (fromSearch) return fromSearch;
    return null;
  };

  const activeBranch = () => {
    if (props.branch) return props.branch;
    const fromSearch = searchParams().get('branch');
    if (fromSearch) return fromSearch;
    return null;
  };

  const loadQueueTicket = async () => {
    const tId = activeTicketId();
    const br = activeBranch();

    if (!tId || !br) {
      setTicket(null);
      setQueueState('empty');
      return;
    }

    setQueueState('loading');
    try {
      const url = `/api/v3/views/my_queue_ticket?branch=${encodeURIComponent(br)}&ticketId=${encodeURIComponent(tId)}`;
      const res = await fetchFn(url, {
        method: 'GET',
        credentials: 'same-origin',
      });

      if (!res.ok) {
        if (res.status === 401) {
          setTicket(null);
          setQueueState('unauthorized');
          return;
        }
        if (res.status === 403 || res.status === 404) {
          setTicket(null);
          setQueueState('empty');
          return;
        }
        setQueueState('error');
        return;
      }

      const json = await res.json();
      const data: MyQueueTicketData = (json && typeof json === 'object' && 'data' in json && json.data) ? json.data : json;
      if (data && data.ticketId) {
        setTicket(data);
        setQueueState('ready');
      } else {
        setTicket(null);
        setQueueState('empty');
      }
    } catch {
      setQueueState('error');
    }
  };

  const loadDrafts = async () => {
    setDraftsState('loading');
    try {
      const res = await fetchFn('/api/v3/views/my_planner_drafts', {
        method: 'GET',
        credentials: 'same-origin',
      });

      if (!res.ok) {
        setDraftsState('error');
        return;
      }

      const json = await res.json();
      const data: MyPlannerDraftsData = (json && typeof json === 'object' && 'data' in json && json.data) ? json.data : json;
      const list = data?.drafts ?? [];
      setDrafts(list);
      setDraftsState(list.length === 0 ? 'empty' : 'ready');
    } catch {
      setDraftsState('error');
    }
  };

  const refreshAll = () => {
    void loadQueueTicket();
    void loadDrafts();
  };

  onMount(() => {
    refreshAll();
  });

  const waitEstimateText = () => {
    const tkt = ticket();
    if (!tkt) return '';
    if (tkt.estimateSeconds === null || tkt.estimateSeconds === undefined) {
      return t('unknown');
    }
    if (
      tkt.estimateLowSeconds !== null &&
      tkt.estimateLowSeconds !== undefined &&
      tkt.estimateHighSeconds !== null &&
      tkt.estimateHighSeconds !== undefined
    ) {
      const lowMin = Math.round(tkt.estimateLowSeconds / 60);
      const highMin = Math.round(tkt.estimateHighSeconds / 60);
      return t('waitRange', { low: lowMin, high: highMin });
    }
    const min = Math.round(tkt.estimateSeconds / 60);
    return t('waitMinutes', { minutes: min });
  };

  const statusLabel = (status: string) => {
    switch (status) {
      case 'WAITING':
        return t('statusWaiting');
      case 'CALLED':
        return t('statusCalled');
      case 'SERVING':
        return t('statusServing');
      case 'COMPLETED':
        return t('statusCompleted');
      case 'NO_SHOW':
        return t('statusNoShow');
      case 'CANCELLED':
        return t('statusCancelled');
      default:
        return status;
    }
  };

  const statusBadgeClass = (status: string) => {
    switch (status) {
      case 'WAITING':
        return 'xc-journey-badge xc-journey-badge-waiting';
      case 'CALLED':
        return 'xc-journey-badge xc-journey-badge-called';
      case 'SERVING':
        return 'xc-journey-badge xc-journey-badge-serving';
      case 'COMPLETED':
        return 'xc-journey-badge xc-journey-badge-completed';
      case 'NO_SHOW':
        return 'xc-journey-badge xc-journey-badge-no-show';
      case 'CANCELLED':
        return 'xc-journey-badge xc-journey-badge-cancelled';
      default:
        return 'xc-journey-badge';
    }
  };

  const formatDate = (iso: string) => {
    try {
      const d = new Date(iso);
      if (isNaN(d.getTime())) return iso;
      return d.toLocaleDateString(locale(), {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        timeZone: 'Asia/Bangkok',
      });
    } catch {
      return iso;
    }
  };

  return (
    <div class="xc-journey-screen">
      <header class="xc-journey-header">
        <div class="xc-journey-title-group">
          <h1>{t('title')}</h1>
          <p class="xc-journey-subtitle">{t('subtitle')}</p>
        </div>
      </header>

      {/* Queue Ticket Section */}
      <section class="xc-journey-section" aria-labelledby="queue-section-heading">
        <div class="xc-journey-section-header">
          <h2 id="queue-section-heading" class="xc-journey-section-title">
            {t('queueTitle')}
          </h2>
          <button
            type="button"
            class="xc-journey-refresh-btn"
            onClick={loadQueueTicket}
            aria-label={t('refresh')}
          >
            {t('refresh')}
          </button>
        </div>

        <Show when={queueState() === 'loading'}>
          <div class="xc-journey-loading" role="status">
            {t('loadingQueue')}
          </div>
        </Show>

        <Show when={queueState() === 'error'}>
          <div class="xc-journey-error" role="alert">
            <span>{t('errorQueue')}</span>
            <button
              type="button"
              class="xc-journey-retry-btn"
              onClick={loadQueueTicket}
            >
              {t('retry')}
            </button>
          </div>
        </Show>

        <Show when={queueState() === 'unauthorized'}>
          <div class="xc-journey-empty" data-testid="queue-unauthorized">
            {t('signInToViewTicket')}
          </div>
        </Show>

        <Show when={queueState() === 'empty'}>
          <div class="xc-journey-empty" data-testid="queue-empty">
            {t('noQueueTicket')}
          </div>
        </Show>

        <Show when={queueState() === 'ready' && ticket()}>
          {(() => {
            const tkt = ticket()!;
            return (
              <div class="xc-journey-card xc-journey-ticket-card" data-testid="ticket-card">
                <div class="xc-journey-ticket-header">
                  <div>
                    <div class="xc-journey-field-label">{t('ticketNumber')}</div>
                    <div class="xc-journey-ticket-number" data-testid="ticket-number">
                      {tkt.displayNumber}
                    </div>
                  </div>
                  <span class={statusBadgeClass(tkt.status)} data-testid="ticket-status">
                    {statusLabel(tkt.status)}
                  </span>
                </div>

                <div class="xc-journey-grid">
                  <div class="xc-journey-field">
                    <span class="xc-journey-field-label">{t('branch')}</span>
                    <span class="xc-journey-field-value ticket-branch">{tkt.branch}</span>
                  </div>

                  <div class="xc-journey-field">
                    <span class="xc-journey-field-label">{t('serviceType')}</span>
                    <span class="xc-journey-field-value ticket-service-type">
                      {tkt.serviceType === 'WALK_IN' ? t('walkIn') : t('booking')}
                    </span>
                  </div>

                  <div class="xc-journey-field">
                    <span class="xc-journey-field-label">{t('ticketsAhead')}</span>
                    <span class="xc-journey-field-value tickets-ahead" data-testid="tickets-ahead">
                      {t('ticketsAheadCount', { count: tkt.ticketsAhead })}
                    </span>
                  </div>

                  <div class="xc-journey-field">
                    <span class="xc-journey-field-label">{t('estimatedWait')}</span>
                    <span class="xc-journey-field-value wait-estimate" data-testid="wait-estimate">
                      {waitEstimateText()}
                    </span>
                  </div>
                </div>
              </div>
            );
          })()}
        </Show>
      </section>

      {/* Saved Plans Section */}
      <section class="xc-journey-section" aria-labelledby="plans-section-heading">
        <div class="xc-journey-section-header">
          <h2 id="plans-section-heading" class="xc-journey-section-title">
            {t('plansTitle')}
          </h2>
          <button
            type="button"
            class="xc-journey-refresh-btn"
            onClick={loadDrafts}
            aria-label={t('refresh')}
          >
            {t('refresh')}
          </button>
        </div>

        <Show when={draftsState() === 'loading'}>
          <div class="xc-journey-loading" role="status">
            {t('loadingPlans')}
          </div>
        </Show>

        <Show when={draftsState() === 'error'}>
          <div class="xc-journey-error" role="alert">
            <span>{t('errorPlans')}</span>
            <button
              type="button"
              class="xc-journey-retry-btn"
              onClick={loadDrafts}
            >
              {t('retry')}
            </button>
          </div>
        </Show>

        <Show when={draftsState() === 'empty'}>
          <div class="xc-journey-empty" data-testid="plans-empty">
            {t('emptyPlans')}
          </div>
        </Show>

        <Show when={draftsState() === 'ready'}>
          <div class="xc-journey-drafts-list" role="list">
            <For each={drafts()}>
              {(draft) => (
                <div class="xc-journey-draft-card" role="listitem" data-testid="draft-card">
                  <div class="xc-journey-draft-header">
                    <span class="xc-journey-draft-side">
                      {draft.customerSide === 'CUSTOMER_BUY' ? t('customerBuy') : t('customerSell')}
                    </span>
                    <span class="xc-journey-draft-currency">{draft.currency}</span>
                  </div>

                  <div class="xc-journey-draft-amount">
                    <Show
                      when={'quantity' in draft.amount}
                      fallback={
                        <MoneyText
                          value={(draft.amount as { budgetThb: string }).budgetThb}
                          currency="THB"
                        />
                      }
                    >
                      <MoneyText
                        value={(draft.amount as { quantity: string }).quantity}
                        currency={draft.currency}
                      />
                    </Show>
                  </div>

                  <div class="xc-journey-draft-footer">
                    <Show when={draft.branch}>
                      <span class="draft-branch">{t('branchLabel', { branch: draft.branch! })}</span>
                    </Show>
                    <span class="draft-expires">{t('expiresAt', { date: formatDate(draft.expiresAt) })}</span>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
      </section>
    </div>
  );
}
