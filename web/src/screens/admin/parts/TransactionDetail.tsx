// V2-parts (#476): LIVE exchange detail with slip and receipt sections (ticket §7, design §12.2, §12.14, P2, P6)

import {
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { formatRate } from '../../../shared/ui/rate';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { en, th } from './i18n';
import { methodName, statusName, typeName } from './transaction';
import { formatBangkokTime } from './TransactionTable';
import './parts.css';

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
        if (err instanceof ApiError && err.code === 'signIn') {
          setReceiptState({ status: 'error', message: t('receiptLoadFailed') });
        } else {
          setReceiptState({ status: 'error', message: wordedError(err) });
        }
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
      {/* 1. Dialog note on top */}
      <p class="dialog-note">{t('liveNote')}</p>

      {/* 2. Fields dl */}
      <dl class="details">
        <div>
          <dt>{t('detailId')}</dt>
          <dd>{props.row.id}</dd>
        </div>

        <div>
          <dt>{t('headerStatus')}</dt>
          <dd>{statusName(props.row.status)}</dd>
        </div>

        <div>
          <dt>{t('headerCustomer')}</dt>
          <dd>{props.row.customer}</dd>
        </div>

        <div>
          <dt>{t('detailDateTime')}</dt>
          <dd>
            {props.row.date} {formatBangkokTime(props.row.created)}
          </dd>
        </div>

        <div>
          <dt>{t('headerType')}</dt>
          <dd>{typeName(props.row.type)}</dd>
        </div>

        <div>
          <dt>{t('headerAmount')}</dt>
          <dd>
            {formatMoney(props.row.amount)} {props.row.currency}
          </dd>
        </div>

        <div>
          <dt>{t('detailRate')}</dt>
          <dd>{formatRate(props.row.rate, props.row.unitSize, props.row.currency)}</dd>
        </div>

        <div>
          <dt>{t('detailPrincipal')}</dt>
          <dd>{formatMoney(props.row.principal)} THB</dd>
        </div>

        <div>
          <dt>{t('detailFee')}</dt>
          <dd>{formatMoney(props.row.fee)} THB</dd>
        </div>

        {props.row.type === 'BUY' ? (
          <div>
            <dt>{t('detailSettlementBuy')}</dt>
            <dd>{formatMoney(props.row.settlement)} THB</dd>
          </div>
        ) : (
          <div>
            <dt>{t('detailSettlementSell')}</dt>
            <dd>{formatMoney(props.row.settlement)} THB</dd>
          </div>
        )}

        <div>
          <dt>{t('detailBankFee')}</dt>
          <dd>{formatMoney(props.row.bankFee)} THB</dd>
        </div>

        <div>
          <dt>{t('detailMethod')}</dt>
          <dd>{methodName(props.row.method)}</dd>
        </div>

        <div>
          <dt>{t('detailStaff')}</dt>
          <dd>{props.row.staff}</dd>
        </div>

        <div>
          <dt>{t('detailNotes')}</dt>
          <dd>{props.row.notes ?? '—'}</dd>
        </div>

        <Show when={props.row.referenceRate !== null}>
          <div>
            <dt>{t('detailRefRate')}</dt>
            <dd>{formatRate(props.row.referenceRate!, props.row.unitSize, props.row.currency)}</dd>
          </div>

          <div>
            <dt>{t('detailRefValue')}</dt>
            <dd>{formatMoney(props.row.referenceValue!)} THB</dd>
          </div>

          <div>
            <dt>{t('detailRefSpread')}</dt>
            <dd>
              {formatMoney(props.row.referenceSpread!)} THB (
              {props.row.referencePercent!}%)
            </dd>
          </div>
        </Show>

        <Show when={props.row.cancelReason !== null}>
          <div>
            <dt>{t('detailCancelReason')}</dt>
            <dd>{props.row.cancelReason}</dd>
          </div>
        </Show>
      </dl>

      {/* 3. Actions */}
      <div class="dialog-actions">
        <Show when={props.row.status === 'COMPLETED'}>
          <button type="button" disabled class="danger">
            {t('realTransactionVerifyRefund')}
          </button>
        </Show>
        <button type="button" onClick={props.onClose} class="secondary">
          {t('close')}
        </button>
      </div>

      {/* 4. Slip section */}
      <section class="slip-section">
        <h3>{t('slipSectionTitle')}</h3>
        <Show
          when={props.row.slipUrl}
          fallback={<p>{t('noSlipAttached')}</p>}
        >
          <a
            class="secondary"
            href={props.row.slipUrl!}
            target="_blank"
            rel="noopener"
          >
            {t('openSlip')}
          </a>{' '}
          <a
            class="text-button"
            href={`${props.row.slipUrl!}?download=1`}
          >
            {t('download')}
          </a>
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
            fallback={
              receiptState().status === 'error' ? (
                <p role="alert">
                  {(receiptState() as { status: 'error'; message: string }).message}
                </p>
              ) : (
                <p>{t('receiptLoadFailed')}</p>
              )
            }
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
