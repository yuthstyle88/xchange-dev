// TD-3 #1291: number, status and times (D3); the approval request's status while PENDING_APPROVAL.
import { Show, type JSX } from 'solid-js';
import type { TransactionDetailDto } from '../../shared/api/generated/transaction';
import { formatBangkokDateTime } from '../../shared/format/bangkok-time';
import { Field } from './Field';
import { ht } from './header-texts';
import { t } from './texts';

function tm(v: string | null | undefined): string | null | undefined {
  return v ? formatBangkokDateTime(v) : v;
}

export function statusName(status: string): string {
  switch (status) {
    case 'COMPLETED':
      return t('statusCompleted');
    case 'CANCELLED':
      return t('statusCancelled');
    default:
      return t('statusPending');
  }
}

export function statusText(tx: TransactionDetailDto): string {
  const base = statusName(tx.status);
  const rev = tx.reversal;
  if (rev?.status === 'POSTED') {
    const timePart = rev.postedAt ? ` ${formatBangkokDateTime(rev.postedAt)}` : '';
    return `${base} · ${ht('headerReversed')}${timePart}`;
  }
  if (rev?.status === 'REQUESTED' || rev?.status === 'APPROVED') {
    return `${base} · ${ht('headerReversalPending')}`;
  }
  return base;
}

export function HeaderSection(props: { tx: TransactionDetailDto }): JSX.Element {
  return (
    <section class="xc-dtx-section" data-section="header">
      <dl class="xc-dtx-fields">
        <Field label={t('number')} value={props.tx.documentRef} />
        <Field label={t('status')} value={statusText(props.tx)} />
        <Field label={t('completedAt')} value={tm(props.tx.completedAt)} />
        <Show when={props.tx.backdate}>
          {(b) => (
            <Field
              label={ht('headerBackdated')}
              value={`${ht('headerBackdatedEntered')} ${formatBangkokDateTime(b().enteredAt)}`}
            />
          )}
        </Show>
        <Show when={props.tx.status === 'CANCELLED'}>
          <Field label={t('cancelledAt')} value={tm(props.tx.cancelledAt)} />
          <Field label={t('cancelReason')} value={props.tx.cancelReason} />
        </Show>
        <Show when={props.tx.status === 'PENDING_APPROVAL' && props.tx.approvalRequest}>
          {(approval) => <Field label={t('approval')} value={approval().status} />}
        </Show>
      </dl>
    </section>
  );
}
