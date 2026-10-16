import { For, Show, createSignal, onMount, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { MyTransactionItem, MyTransactionsData } from '../../../shared/api/generated/my_transactions';
import { lang, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { MoneyText } from '../../../shared/ui/MoneyText';
import * as shellTable from '../_shell/i18n';
import { en, th, zh } from './i18n';
import './history.css';

export type TransactionRow = MyTransactionItem;

export interface HistoryScreenProps {
  fetch?: (url: string, init?: RequestInit) => Promise<Response>;
  navigate?: (url: string) => void;
}

export function HistoryScreen(props: HistoryScreenProps): JSX.Element {
  const t = screenT('customer.history', { th, en, zh });
  const [loading, setLoading] = createSignal(true);
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);
  const [unauthorized, setUnauthorized] = createSignal(false);
  const [transactions, setTransactions] = createSignal<TransactionRow[]>([]);

  const api = new ApiClient(props.fetch);

  const routeToSignIn = () => {
    if (props.navigate) {
      props.navigate('/app/sign-in');
    } else if (typeof window !== 'undefined' && typeof window.location?.assign === 'function') {
      try {
        window.location.assign('/app/sign-in');
      } catch {
        // ignore navigation error in non-browser test runners
      }
    }
  };

  const loadData = async () => {
    setLoading(true);
    setErrorMsg(null);
    setUnauthorized(false);
    try {
      const answer = await api.view<MyTransactionsData>('my_transactions');
      const items = answer.data?.transactions ?? [];
      setTransactions(items);
    } catch (err: unknown) {
      if (
        (err instanceof ApiError && err.status === 401) ||
        (typeof err === 'object' && err !== null && 'status' in err && (err as { status: unknown }).status === 401)
      ) {
        setUnauthorized(true);
        routeToSignIn();
        return;
      }
      setErrorMsg(wordedError(err));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    void loadData();
  });

  return (
    <div class="xc-history-screen">
      <Show when={loading()}>
        <div class="xc-history-loading" role="status">
          {t('loading')}
        </div>
      </Show>

      <Show when={!loading() && unauthorized()}>
        <a class="xc-customer-signin" href="/app/sign-in">
          {shellTable[lang()].signIn}
        </a>
      </Show>

      <Show when={!loading() && errorMsg()}>
        {(msg) => (
          <div class="xc-history-error" role="alert">
            <p>{t('loadFailed')}</p>
            <p>{msg()}</p>
            <button type="button" class="xc-history-retry-btn" onClick={loadData}>
              {t('retry')}
            </button>
          </div>
        )}
      </Show>

      <Show when={!loading() && !errorMsg() && !unauthorized() && transactions().length === 0}>
        <div class="xc-history-empty">{t('empty')}</div>
      </Show>

      <Show when={!loading() && !errorMsg() && !unauthorized() && transactions().length > 0}>
        <div class="xc-history-list">
          <For each={transactions()}>
            {(tx) => (
              <div class="xc-history-item">
                <div class="xc-history-item-header">
                  <span class="xc-history-date">{tx.date}</span>
                  <span class="xc-history-branch">{tx.branchName || tx.branch}</span>
                  <span
                    class={`xc-history-status ${tx.status === 'REVERSED' ? 'xc-history-status-reversed' : ''}`}
                  >
                    {tx.status === 'REVERSED' ? t('status_reversed') : t('status_completed')}
                  </span>
                </div>
                <div class="xc-history-item-details">
                  <div class="xc-history-amount-row">
                    <span class="xc-history-type">{tx.type}</span>
                    <MoneyText value={tx.amount} currency={tx.currency} />
                  </div>
                  <div class="xc-history-settlement-row">
                    <span class="xc-history-label">{t('settlement')}:</span>
                    <MoneyText value={tx.settlement} currency="THB" />
                  </div>
                </div>
                <Show when={tx.receiptRef}>
                  <div class="xc-history-receipt-row">
                    <a
                      class="xc-history-receipt-link"
                      href={`/api/app/receipts/${encodeURIComponent(tx.receiptRef)}`}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      {t('receipt')}: {tx.receiptRef}
                    </a>
                  </div>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
