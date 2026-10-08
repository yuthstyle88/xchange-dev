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
