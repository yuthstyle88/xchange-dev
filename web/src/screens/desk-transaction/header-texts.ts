// FE-T6 #5854: header reversal status text.
import { screenT } from '../../shared/i18n';

export const th = {
  headerReversed: 'กลับรายการแล้ว',
  headerReversalPending: 'รอกลับรายการ',
  headerBackdated: 'ย้อนหลัง',
  headerBackdatedEntered: 'บันทึกเมื่อ',
};

export const en: Record<keyof typeof th, string> = {
  headerReversed: 'Reversed',
  headerReversalPending: 'Reversal pending',
  headerBackdated: 'Backdated',
  headerBackdatedEntered: 'Entered',
};

export const ht = screenT('desk-transaction', { th, en });
