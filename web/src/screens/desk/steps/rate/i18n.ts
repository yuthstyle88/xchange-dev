// Step 0: v2's Thai copy verbatim (design xchange-v3-wave3-desk.md §3.6b row "Step 0 rate", dh:114,170,176).
export const th = {
  label: 'เลือกเรท',
  heading: 'เลือกเรทและจำนวนเงิน',
  next: 'ถัดไป · พาสปอร์ต / ลูกค้า →',
  noShift: 'ยังไม่มีกะเปิด กรุณาเปิดกะจากหน้าแอดมินก่อน',
  needShiftAndRate: 'ต้องเปิดกะและตั้งเรทสกุลเงินนี้ในหน้าแอดมินก่อน จากนั้นโหลดหน้านี้ใหม่',
  type: 'ประเภทรายการ',
  typeBuy: 'ร้านรับซื้อ (BUY)',
  typeSell: 'ร้านขาย (SELL)',
  currency: 'สกุลเงิน',
  amount: 'จำนวนเงิน',
  rate: 'เรทหน้าร้านที่ตั้งใน เซิร์ฟเวอร์',
  perUnit: 'ต่อ {unit} {code}',
  referenceRate: 'เรทอ้างอิง (ไม่บังคับ)',
  quoting: 'กำลังคำนวณใหม่…',
} as const;

export const en: Record<keyof typeof th, string> = {
  label: 'Rate',
  heading: 'Choose the rate and amount',
  next: 'Next · Passport / customer →',
  noShift: 'No shift is open. Please open a shift in the admin first',
  needShiftAndRate: 'Open a shift and set a rate for this currency in the admin first, then reload this page',
  type: 'Transaction type',
  typeBuy: 'Shop buys (BUY)',
  typeSell: 'Shop sells (SELL)',
  currency: 'Currency',
  amount: 'Amount',
  rate: 'Counter rate set on the server',
  perUnit: 'per {unit} {code}',
  referenceRate: 'Reference rate (optional)',
  quoting: 'Recalculating…',
};
