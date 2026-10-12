import { For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../../shared/i18n';
import { ConfirmDialog } from '../../../staff/parts/ConfirmDialog';
import * as texts from './i18n';
import type { RefundsStore } from './refunds-store';
import './refunds.css';

const t = screenT('desk.steps.refunds', texts);

export function RefundsStep(props: { store: RefundsStore }): JSX.Element {
  const store = props.store;

  return (
    <div data-section="refunds" class="desk-refunds-section">
      <div class="desk-refunds-toolbar">
        <h2 class="desk-refunds-title">{t('title')}</h2>
        <div class="desk-refunds-toolbar-actions">
          <button
            type="button"
            data-action="refresh"
            class="desk-refunds-btn"
            onClick={() => void store.readRefunds()}
          >
            {t('refresh')}
          </button>
          <button
            type="button"
            data-action="close"
            class="desk-refunds-btn"
            onClick={() => store.setOpen(false)}
          >
            {t('close')}
          </button>
        </div>
      </div>

      <Show when={store.lastPostedDoc()}>
        {(doc) => (
          <div class="desk-refunds-success">
            <span class="desk-refunds-doc">{doc()}</span>
            <button
              type="button"
              data-action="print"
              class="desk-refunds-btn desk-refunds-print"
              onClick={() => void store.printReceipt()}
            >
              {t('print')}
            </button>
          </div>
        )}
      </Show>

      <Show when={store.postError()}>
        <div class="desk-refunds-alert">
          <span class="desk-refunds-error-msg">{store.postError()}</span>
          <Show when={store.canRetry()}>
            <button
              type="button"
              data-action="retry"
              class="desk-refunds-btn desk-refunds-retry"
              onClick={() => void store.retry()}
            >
              {t('retry')}
            </button>
          </Show>
        </div>
      </Show>

      <Show
        when={store.rows().length > 0}
        fallback={<div class="desk-refunds-empty">{t('empty')}</div>}
      >
        <div class="desk-refunds-list">
          <For each={store.rows()}>
            {(row) => (
              <div data-reversal={row.reversal.id} class="desk-refund-row">
                <div class="desk-refund-row-header">
                  <span class="desk-refund-receipt-no">{row.receiptNumber}</span>
                  <span class="desk-refund-amount">{row.reversal.amount}</span>
                  <span class="desk-refund-reason">{row.reversal.reason}</span>
                  <a
                    href={`/desk/transactions/${row.reversal.originalTransactionId}`}
                    class="desk-refund-link"
                  >
                    {t('originalTx')}
                  </a>
                  <button
                    type="button"
                    data-action="post"
                    data-id={row.reversal.id}
                    class="desk-refund-post-btn"
                    onClick={() => store.startConfirm(row.reversal.id)}
                  >
                    {t('post')}
                  </button>
                </div>

                <div class="desk-refund-lines">
                  <For each={row.lines}>
                    {(line) => (
                      <div class="desk-refund-line">
                        <span class="desk-refund-side">{line.shopSide}</span>
                        <span class="desk-refund-currency">{line.currency}</span>
                        <span class="desk-refund-qty">{line.quantity}</span>
                        <span class="desk-refund-settle">{line.settleThb}</span>
                      </div>
                    )}
                  </For>
                </div>

                <div class="desk-refund-payments">
                  <For each={row.payments}>
                    {(pm) => (
                      <div class="desk-refund-payment">
                        <span class="desk-refund-method">{pm.method}</span>
                        <span class="desk-refund-pay-amount">{pm.amountThb}</span>
                      </div>
                    )}
                  </For>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <ConfirmDialog
        open={store.confirmingId() !== null}
        title={t('confirmTitle')}
        message={t('confirmMessage')}
        confirmLabel={t('confirm')}
        cancelLabel={t('cancel')}
        onConfirm={() => void store.submitPost()}
        onCancel={() => store.cancelConfirm()}
      />
    </div>
  );
}
