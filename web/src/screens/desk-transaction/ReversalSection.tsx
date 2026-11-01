// TD-3 #1291: the reversal (D6). The request goes through S-3's ReasonDialog; there is no post here: an APPROVED
// reversal is posted at the desk (owner Q-1 B), so this section only says so.
import { Show, type JSX } from 'solid-js';
import type { ReversalSummaryDto } from '../../shared/api/generated/reversal';
import { formatBangkokDateTime } from '../../shared/format/bangkok-time';
import { Field } from './Field';
import { t } from './texts';

function tm(v: string | null | undefined): string | null | undefined {
  return v ? formatBangkokDateTime(v) : v;
}

export function ReversalSection(props: { reversal: ReversalSummaryDto }): JSX.Element {
  return (
    <section class="xc-dtx-section" data-section="reversal">
      <h2 class="xc-dtx-heading">{t('reversal')}</h2>
      <dl class="xc-dtx-fields">
        <Field label={t('status')} value={props.reversal.status} />
        <Field label={t('reason')} value={props.reversal.reason} />
        <Field label={t('amount')} value={props.reversal.amount} />
        <Field label={t('requestedBy')} value={props.reversal.requestedBy} />
        <Field label={t('requestedAt')} value={tm(props.reversal.requestedAt)} />
        <Field label={t('approvedAt')} value={tm(props.reversal.approvedAt)} />
        <Field label={t('postedAt')} value={tm(props.reversal.postedAt)} />
        <Field label={t('documentNumber')} value={props.reversal.documentNumber} />
      </dl>
      <Show when={props.reversal.status === 'APPROVED'}>
        <p class="xc-dtx-note">{t('approvedAtDesk')}</p>
      </Show>
    </section>
  );
}
