// Pricing formula FE (docs/design/blueprint/pricing-formula-web.md §1): the `pricing-formula` menu's own text.
// Thai only; `en` is a copy of `th` (staff screens are TH/EN only) until the translation ticket (owner 2026-10-28).

export const th = {
  label: 'สูตรคำนวณราคา',
  eyebrow: 'PRICING FORMULA',
  title: 'สูตรคำนวณราคาขององค์กร',
  subtitle: 'กำหนดสูตรราคารับซื้อและราคาขายที่ทุกสาขาใช้ เว้นแต่สาขาจะมีสูตรของตัวเอง',

  loading: 'กำลังโหลดสูตร…',
  retry: 'ลองใหม่',
  currentVersion: 'สูตรองค์กรฉบับที่ {version} อนุมัติโดย {approvedBy} เมื่อ {validFrom}',
  noOrganisation: 'ยังไม่มีสูตรขององค์กร ตอนนี้ทุกสาขาใช้สูตรมาตรฐานของระบบ',
  branchesTitle: 'สาขาใช้สูตรไหน',
  colBranch: 'สาขา',
  colSource: 'สูตรที่ใช้',
  colOwnText: 'สูตรของสาขา',
  reasonLabel: 'เหตุผลที่เปลี่ยนสูตร',
  reasonRequired: 'กรุณาระบุเหตุผล',
  textRequired: 'กรอกสูตรให้ครบทั้งราคารับซื้อและราคาขาย',
  save: 'บันทึกสูตรองค์กร',
  saving: 'กำลังบันทึก…',
  saved: 'บันทึกสูตรองค์กรแล้ว',
};

export const en: Record<keyof typeof th, string> = { ...th };
