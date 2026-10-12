// TD-3 #1291: number, status and times (D3); the approval request's status while PENDING_APPROVAL.
import { Show, type JSX } from 'solid-js';
import type { TransactionDetailDto } from '../../shared/api/generated/transaction';
import { Field } from './Field';
import { t } from './texts';

export function HeaderSection(props: { tx: TransactionDetailDto }): JSX.Element {
  return (
    <section class="xc-dtx-section" data-section="header">
      <dl class="xc-dtx-fields">
        <Field label={t('number')} value={props.tx.documentRef} />
        <Field label={t('status')} value={props.tx.status} />
        <Field label={t('completedAt')} value={props.tx.completedAt} />
        <Show when={props.tx.status === 'CANCELLED'}>
          <Field label={t('cancelledAt')} value={props.tx.cancelledAt} />
          <Field label={t('cancelReason')} value={props.tx.cancelReason} />
        </Show>
        <Show when={props.tx.status === 'PENDING_APPROVAL' && props.tx.approvalRequest}>
          {(approval) => <Field label={t('approval')} value={approval().status} />}
        </Show>
      </dl>
    </section>
  );
}
