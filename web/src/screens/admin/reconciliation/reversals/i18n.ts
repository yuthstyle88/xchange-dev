// FE-admin (#3333), survey row A4: pending reversals part texts, namespace admin.reconciliation.reversals.
export const th = {
  approve: 'อนุมัติ',
  reject: 'ปฏิเสธ',
  reason: 'เหตุผล',
  reasonRequired: 'กรุณาระบุเหตุผลในการปฏิเสธ',
  reasonTooLong: 'เหตุผลยาวได้ไม่เกิน 500 ตัวอักษร',
};

export const en: Record<keyof typeof th, string> = {
  approve: 'Approve',
  reject: 'Reject',
  reason: 'Reason',
  reasonRequired: 'A reason is required to reject',
  reasonTooLong: 'Reason can be at most 500 characters',
};
