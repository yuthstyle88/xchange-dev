import { For, Show, createSignal, onMount, type JSX } from 'solid-js';
import type { MyTransactionItem, MyTransactionsData } from '../../../shared/api/generated/my_transactions';
import { screenT } from '../../../shared/i18n';
import { MoneyText } from '../../../shared/ui/MoneyText';
import { en, th, zh } from './i18n';
import './history.css';

export type TransactionRow = MyTransactionItem;

export interface HistoryScreenProps {
  fetch?: (url: string, init?: RequestInit) => Promise<Response>;
}

export function HistoryScreen(props: HistoryScreenProps): JSX.Element {
  const t = screenT('customer.history', { th, en, zh });
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal(false);
  const [transactions, setTransactions] = createSignal<TransactionRow[]>([]);

  const fetchFn = props.fetch ?? ((url: string, init?: RequestInit) => fetch(url, init));

  const loadData = async () => {
    setLoading(true);
    setError(false);
    try {
      const res = await fetchFn('/api/v3/views/my_transactions');
      if (!res.ok) {
        setError(true);
        setLoading(false);
        return;
      }
      const json = (await res.json()) as { data?: MyTransactionsData };
      const items = json?.data?.transactions ?? [];
      setTransactions(items);
      setLoading(false);
    } catch {
      setError(true);
      setLoading(false);
    }
  };

  onMount(() => {
    loadData();
  });

  return (
    <div class="xc-history-screen">
      <Show when={loading()}>
        <div class="xc-history-loading" role="status">
          {t('loading')}
        </div>
      </Show>

      <Show when={!loading() && error()}>
        <div class="xc-history-error">
          <p>{t('loadFailed')}</p>
          <button type="button" class="xc-history-retry-btn" onClick={loadData}>
            {t('retry')}
          </button>
        </div>
      </Show>

      <Show when={!loading() && !error() && transactions().length === 0}>
        <div class="xc-history-empty">{t('empty')}</div>
      </Show>

      <Show when={!loading() && !error() && transactions().length > 0}>
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
