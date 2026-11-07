// TD-3 #1291: the payment (D3).
import type { JSX } from 'solid-js';
import type { TxPaymentDto } from '../../shared/api/generated/transaction';
import { formatBangkokDateTime } from '../../shared/format/bangkok-time';
import { Field } from './Field';
import { t } from './texts';

function tm(v: string | null | undefined): string | null | undefined {
  return v ? formatBangkokDateTime(v) : v;
}

export function methodName(method: string): string {
  switch (method) {
    case 'Cash':
      return t('methodCash');
    case 'Credit Card':
      return t('methodCreditCard');
    case 'Alipay':
      return 'Alipay';
    case 'PromptPay':
      return t('methodPromptPay');
    case 'Bank Transfer':
    case 'SCB Transfer':
      return t('methodBankTransfer');
    default:
      return t('methodOther');
  }
}

export function PaymentSection(props: { payment: TxPaymentDto }): JSX.Element {
  return (
    <section class="xc-dtx-section" data-section="payment">
      <h2 class="xc-dtx-heading">{t('payment')}</h2>
      <dl class="xc-dtx-fields">
        <Field label={t('method')} value={methodName(props.payment.method)} />
        <Field label={t('reference')} value={props.payment.reference} />
        <Field label={t('account')} value={props.payment.account} />
        <Field label={t('amountThb')} value={props.payment.amountThb} />
        <Field label={t('bankFeeThb')} value={props.payment.bankFeeThb} />
        <Field label={t('paidAt')} value={tm(props.payment.paidAt)} />
      </dl>
    </section>
  );
}
