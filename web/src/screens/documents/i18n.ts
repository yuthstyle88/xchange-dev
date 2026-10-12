// #1349 V3-documents (design §A D-A7): the documents pages' texts. The Thai values are v2's words verbatim.
export const th = {
  title: 'เอกสารรายการ',
  refLine: '{ref} · ใบเสร็จและพาสปอร์ตผูกกับรหัสออเดอร์เดียวกัน',
  open: 'เปิดดู',
  download: 'ดาวน์โหลด {name}',
  backOwner: 'กลับรายการแลกเงิน',
  backStaff: 'กลับหน้าพนักงาน',
  loading: 'กำลังโหลด…',
  'kind.passport': 'พาสปอร์ต',
  'kind.paymentProof': 'หลักฐานชำระเงิน',
  'kind.receipt': 'ใบเสร็จ',
  badBranch: 'สาขาไม่ถูกต้อง',
};

export const en: Record<keyof typeof th, string> = {
  title: 'Exchange documents',
  refLine: '{ref} · the receipt and the passport share one order number',
  open: 'Open',
  download: 'Download {name}',
  backOwner: 'Back to exchanges',
  backStaff: 'Back to the staff page',
  loading: 'Loading…',
  'kind.passport': 'Passport',
  'kind.paymentProof': 'Payment proof',
  'kind.receipt': 'Receipt',
  badBranch: 'Invalid branch',
};
