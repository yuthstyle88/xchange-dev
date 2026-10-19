// FE-admin (#3333), survey row A4: pending reversals part texts, namespace admin.reconciliation.reversals.
export const th = {
  approve: 'อนุมัติ',
  reject: 'ปฏิเสธ',
  reason: 'เหตุผล',
  reasonRequired: 'กรุณาระบุเหตุผลในการปฏิเสธ',
};

export const en: Record<keyof typeof th, string> = {
  approve: 'Approve',
  reject: 'Reject',
  reason: 'Reason',
  reasonRequired: 'A reason is required to reject',
};
