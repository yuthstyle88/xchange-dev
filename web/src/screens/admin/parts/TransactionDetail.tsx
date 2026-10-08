// V2-parts (#476): LIVE exchange detail with slip and receipt sections (ticket §7, design §12.2, §12.14, P2, P6)

import {
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { screenT } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { en, th } from './i18n';
import { methodName, statusName, typeName } from './transaction';
import { formatBangkokTime } from './TransactionTable';

export interface TransactionDetailProps {
  api: ApiClient;
  branch: string;
  row: TransactionRow;
  onClose: () => void;
}

interface DocumentFile {
  kind: 'receipt' | 'receiptImage';
  path: string;
  download?: string;
}

interface DocumentsAnswer {
  ref: string;
  files: DocumentFile[];
}

export function TransactionDetail(props: TransactionDetailProps): JSX.Element {
  const t = screenT('admin.parts', { th, en });

  const [receiptState, setReceiptState] = createSignal<
    | { status: 'loading' }
    | { status: 'error'; message: string }
    | { status: 'loaded'; files: DocumentFile[] }
  >({ status: 'loading' });

  let isDisposed = false;

  onMount(() => {
    props.api
      .view<DocumentsAnswer>('transaction_documents', {
        branch: props.branch,
        filter: { ref: props.row.id },
      })
      .then((answer) => {
        if (isDisposed) return;
        const allFiles = answer.data?.files ?? [];
        const validFiles = allFiles.filter(
          (f) =>
            typeof f.path === 'string' &&
            f.path.startsWith('/files/'),
        );
        setReceiptState({ status: 'loaded', files: validFiles });
      })
      .catch((err: unknown) => {
        if (isDisposed) return;
        const msg = err instanceof Error ? err.message : String(err);
        setReceiptState({ status: 'error', message: msg });
      });
  });

  onCleanup(() => {
    isDisposed = true;
  });

  const receiptFile = () => {
    const s = receiptState();
    if (s.status === 'loaded') {
      return s.files.find((f) => f.kind === 'receipt');
    }
    return undefined;
  };

  return (
    <div class="transaction-detail">
      {/* 1. Fields dl */}
      <dl class="detail-fields">
        <dt>{t('detailId')}</dt>
        <dd>{props.row.id}</dd>

        <dt>{t('headerStatus')}</dt>
        <dd>{statusName(props.row.status)}</dd>

        <dt>{t('headerCustomer')}</dt>
        <dd>{props.row.customer}</dd>

        <dt>{t('detailDateTime')}</dt>
        <dd>
          {props.row.date} {formatBangkokTime(props.row.created)}
        </dd>

        <dt>{t('headerType')}</dt>
        <dd>{typeName(props.row.type)}</dd>

        <dt>{t('headerAmount')}</dt>
        <dd>
          {formatMoney(props.row.amount)} {props.row.currency}
        </dd>

        <dt>{t('detailRate')}</dt>
        <dd>{props.row.rate}</dd>

        <dt>{t('detailPrincipal')}</dt>
        <dd>{formatMoney(props.row.principal)} THB</dd>

        <dt>{t('detailFee')}</dt>
        <dd>{formatMoney(props.row.fee)} THB</dd>

        {props.row.type === 'BUY' ? (
          <>
            <dt>{t('detailSettlementBuy')}</dt>
            <dd>{formatMoney(props.row.settlement)} THB</dd>
          </>
        ) : (
          <>
            <dt>{t('detailSettlementSell')}</dt>
            <dd>{formatMoney(props.row.settlement)} THB</dd>
          </>
        )}

        <dt>{t('detailBankFee')}</dt>
        <dd>{formatMoney(props.row.bankFee)} THB</dd>

        <dt>{t('detailMethod')}</dt>
        <dd>{methodName(props.row.method)}</dd>

        <dt>{t('detailStaff')}</dt>
        <dd>{props.row.staff}</dd>

        <dt>{t('detailNotes')}</dt>
        <dd>{props.row.notes ?? '—'}</dd>

        <Show when={props.row.referenceRate !== null}>
          <dt>{t('detailRefRate')}</dt>
          <dd>{props.row.referenceRate}</dd>

          <dt>{t('detailRefValue')}</dt>
          <dd>{formatMoney(props.row.referenceValue!)} THB</dd>

          <dt>{t('detailRefSpread')}</dt>
          <dd>
            {formatMoney(props.row.referenceSpread!)} THB (
            {props.row.referencePercent!}%)
          </dd>
        </Show>

        <Show when={props.row.cancelReason !== null}>
          <dt>{t('detailCancelReason')}</dt>
          <dd>{props.row.cancelReason}</dd>
        </Show>
      </dl>

      {/* 2. LIVE note and buttons */}
      <p class="live-note">{t('liveNote')}</p>

      <div class="detail-actions">
        <Show when={props.row.status === 'COMPLETED'}>
          <button type="button" disabled class="btn-cancel-disabled">
            {t('realTransactionVerifyRefund')}
          </button>
        </Show>
        <button type="button" onClick={props.onClose} class="btn-close">
          {t('close')}
        </button>
      </div>

      {/* 3. Slip section */}
      <section class="slip-section">
        <h3>{t('slipSectionTitle')}</h3>
        <Show
          when={props.row.slipUrl}
          fallback={<p>{t('noSlipAttached')}</p>}
        >
          <div class="slip-links">
            <a
              href={props.row.slipUrl!}
              target="_blank"
              rel="noopener"
            >
              {t('openSlip')}
            </a>
            <a href={`${props.row.slipUrl!}?download=1`}>
              {t('download')}
            </a>
          </div>
        </Show>
      </section>

      {/* 4. Receipt section */}
      <section class="receipt-section">
        <h3>{t('receiptHeading')}</h3>
        <Show
          when={receiptState().status !== 'loading'}
          fallback={<p>{t('loadingReceiptFor', { id: props.row.id })}</p>}
        >
          <Show
            when={receiptState().status === 'loaded'}
            fallback={<p>{t('receiptLoadFailed')}</p>}
          >
            {(() => {
              const files = (
                receiptState() as { status: 'loaded'; files: DocumentFile[] }
              ).files;
              return (
                <Show
                  when={files.length > 0}
                  fallback={<p>{t('noReceiptYet')}</p>}
                >
                  <p>{t('savedDocumentsFor', { id: props.row.id })}</p>
                  <div class="receipt-links">
                    <For each={files}>
                      {(file) => (
                        <div class="receipt-file-item">
                          <Show
                            when={file.kind === 'receipt'}
                            fallback={
                              <a
                                href={file.path}
                                target="_blank"
                                rel="noopener"
                              >
                                {t('openReceiptImage')}
                              </a>
                            }
                          >
                            <a
                              href={file.path}
                              target="_blank"
                              rel="noopener"
                            >
                              {t('openReceipt')}
                            </a>
                          </Show>
                          <Show when={file.download}>
                            {' · '}
                            <a href={file.path} download={file.download}>
                              {file.download}
                            </a>
                          </Show>
                        </div>
                      )}
                    </For>
                  </div>
                  <Show when={receiptFile()}>
                    {(file) => (
                      <iframe
                        src={file().path}
                        sandbox=""
                        title={t('receiptTitleFor', { id: props.row.id })}
                      />
                    )}
                  </Show>
                </Show>
              );
            })()}
          </Show>
        </Show>
      </section>
    </div>
  );
}
