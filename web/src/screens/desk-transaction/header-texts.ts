// FE-T6 #5854: header reversal status text.
import { screenT } from '../../shared/i18n';

export const th = {
  headerReversed: 'กลับรายการแล้ว',
  headerReversalPending: 'รอกลับรายการ',
};

export const en: Record<keyof typeof th, string> = {
  headerReversed: 'Reversed',
  headerReversalPending: 'Reversal pending',
};

export const ht = screenT('desk-transaction', { th, en });
