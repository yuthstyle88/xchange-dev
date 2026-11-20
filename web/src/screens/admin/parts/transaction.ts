// V2-parts (#476): label mappings for transactions (ticket §7, design §12.2)

import { screenT } from '../../../shared/i18n';
import { en, th } from './i18n';

const t = screenT('admin.parts', { th, en });

export function typeName(type: string): string {
  if (type === 'BUY') {
    return t('buy');
  }
  return t('sell');
}

export function methodName(method: string): string {
  switch (method) {
    case 'Cash':
      return t('methodCash');
    case 'Credit Card':
      return t('methodCreditCard');
    case 'Alipay':
      return t('methodAlipay');
    case 'PromptPay':
      return t('methodPromptPay');
    case 'Bank Transfer':
    case 'SCB Transfer':
      return t('methodBankTransfer');
    default:
      return t('methodOther');
  }
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

/** A row's status as shown: a COMPLETED exchange undone by a POSTED reversal stays `COMPLETED`
 *  (reversal design V-2), so the server sends `reversedAt` and the screen adds "Reversed". */
export function rowStatusName(row: { status: string; reversedAt: string | null }): string {
  const base = statusName(row.status);
  return row.reversedAt ? `${base} · ${t('statusReversed')}` : base;
}

/** Who served, by name; the user id only when the server knows no name for it. */
export function staffLabel(row: { staff: string; staffName: string | null }): string {
  return row.staffName ?? row.staff;
}
