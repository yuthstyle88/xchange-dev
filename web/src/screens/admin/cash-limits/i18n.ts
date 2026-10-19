// FE-admin (#3324), survey row A22: the count-difference limit screen's texts (ledger-inventory.md §3.3, Q-2).
export const th = {
  label: 'เกณฑ์ยอดนับต่าง',
  eyebrow: 'COUNT DIFFERENCE LIMIT',
  title: 'เกณฑ์ยอดนับต่าง',
  subtitle: 'ยอดต่างเกินเกณฑ์นี้ต้องให้ฝ่ายการเงินอนุมัติ',
  noLimit: 'ไม่จำกัดเกณฑ์: ยอดนับต่างทุกบาทต้องส่งให้ฝ่ายการเงินอนุมัติ',
  current: 'เกณฑ์ปัจจุบัน {limit} THB · ตั้งโดย {updatedBy} · {updatedAt}',
  limitLabel: 'เกณฑ์ยอดนับต่าง (THB)',
  noLimitChoice: 'ไม่จำกัดเกณฑ์ (ส่งทุกยอดต่างให้ฝ่ายการเงิน)',
  save: 'บันทึกเกณฑ์',
  saved: 'บันทึกเกณฑ์ยอดนับต่างแล้ว',
  'error.countDifferenceLimitThb': 'เกณฑ์ยอดนับต่างไม่ถูกต้อง ต้องเป็นตัวเลขตั้งแต่ 0 ขึ้นไปและทศนิยมไม่เกิน 2 ตำแหน่ง',
  'error.roleNotAllowedHere': 'บัญชีนี้ไม่มีสิทธิ์ตั้งเกณฑ์ยอดนับต่าง',
  'error.idempotencyConflict': 'คำขอนี้ถูกส่งไปแล้วด้วยข้อมูลอื่น กรุณาโหลดหน้าใหม่แล้วบันทึกอีกครั้ง',
};

export const en: Record<keyof typeof th, string> = {
  label: 'Count limit',
  eyebrow: 'COUNT DIFFERENCE LIMIT',
  title: 'Count difference limit',
  subtitle: 'A difference over this limit goes to finance for approval',
  noLimit: 'No limit: every difference goes to finance',
  current: 'Current limit {limit} THB · set by {updatedBy} · {updatedAt}',
  limitLabel: 'Count difference limit (THB)',
  noLimitChoice: 'No limit (every difference goes to finance)',
  save: 'Save limit',
  saved: 'Count difference limit saved',
  'error.countDifferenceLimitThb': 'Invalid count difference limit: enter a non-negative amount with at most 2 decimal places',
  'error.roleNotAllowedHere': 'This account may not set the count difference limit',
  'error.idempotencyConflict': 'This request was already sent with other data; reload the page and save again',
};
