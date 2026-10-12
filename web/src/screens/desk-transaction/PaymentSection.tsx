// TD-3 #1291: the payment (D3).
import type { JSX } from 'solid-js';
import type { TxPaymentDto } from '../../shared/api/generated/transaction';
import { Field } from './Field';
import { t } from './texts';

export function PaymentSection(props: { payment: TxPaymentDto }): JSX.Element {
  return (
    <section class="xc-dtx-section" data-section="payment">
      <h2 class="xc-dtx-heading">{t('payment')}</h2>
      <dl class="xc-dtx-fields">
        <Field label={t('method')} value={props.payment.method} />
        <Field label={t('reference')} value={props.payment.reference} />
        <Field label={t('account')} value={props.payment.account} />
        <Field label={t('amountThb')} value={props.payment.amountThb} />
        <Field label={t('bankFeeThb')} value={props.payment.bankFeeThb} />
        <Field label={t('paidAt')} value={props.payment.paidAt} />
      </dl>
    </section>
  );
}
