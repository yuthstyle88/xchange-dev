// CA-9b #1372: the cash-transfer limit policy screen's texts, TH and EN.
export const th = {
  label: 'วงเงินโอนเงินสด',
  eyebrow: 'CASH TRANSFER POLICY',
  title: 'วงเงินโอนเงินสด',
  subtitle: 'วงเงินต่อสกุลที่การโอนเงินสดเกินแล้วต้องได้รับอนุมัติ',
  noLimit: 'ยังไม่ได้ตั้งวงเงิน',
  current: 'ฉบับ {version} · ตั้งโดย {setBy} · {when}',
  headerCurrency: 'สกุลเงิน',
  headerLimit: 'วงเงิน',
  emptyNote: 'ช่องที่เว้นว่างไม่มีวงเงิน',
  save: 'บันทึกวงเงิน',
  saved: 'บันทึกวงเงินแล้ว',
  'error.roleNotAllowedHere': 'บัญชีนี้ไม่มีสิทธิ์ตั้งวงเงินโอนเงินสด',
  'error.idempotencyConflict': 'คำขอนี้ถูกส่งไปแล้วด้วยข้อมูลอื่น กรุณาโหลดหน้าใหม่แล้วบันทึกอีกครั้ง',
  connectionError: 'เกิดข้อผิดพลาดในการเชื่อมต่อ',
};

export const en: Record<keyof typeof th, string> = {
  label: 'Cash transfer limits',
  eyebrow: 'CASH TRANSFER POLICY',
  title: 'Cash transfer limits',
  subtitle: 'The limit per currency above which a cash transfer needs approval',
  noLimit: 'No limit set yet',
  current: 'Version {version} · set by {setBy} · {when}',
  headerCurrency: 'Currency',
  headerLimit: 'Limit',
  emptyNote: 'A blank row has no limit',
  save: 'Save limits',
  saved: 'Limits saved',
  'error.roleNotAllowedHere': 'This account may not set the cash transfer limits',
  'error.idempotencyConflict': 'This request was already sent with other data; reload the page and save again',
  connectionError: 'Connection error',
};
