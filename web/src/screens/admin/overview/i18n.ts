// V2-overview (#461): overview screen i18n table (W6, W10)

export const th = {
  // Menu & heading
  menuLabel: 'ภาพรวม',
  eyebrow: 'DAILY OVERVIEW',
  title: 'ภาพรวมหน้าร้าน',
  subtitle: 'ติดตามสถานะกะและยอดรายการในช่วงวันที่ที่เลือก',

  // Stat cards
  totalVolume: 'ยอดแลกเงินรวม',
  totalVolumeNote: 'มูลค่าเงินต้นของรายการสำเร็จ',
  buyCardLabel: 'ร้านรับซื้อ',
  buyCardNote: '{count} รายการ · ร้านจ่ายเงินบาท',
  sellCardLabel: 'ร้านขาย',
  sellCardNote: '{count} รายการ · ร้านรับเงินบาท',
  completedTransactions: 'รายการสำเร็จ',
  completedUnit: 'รายการ',
  completedNote: 'ยกเลิก {cancelled} · ค้าง {pending} รายการ',

  // Activity
  activityEyebrow: 'EXCHANGE ACTIVITY',
  activityTitle: 'สัดส่วนยอดแลกเงิน',
  activityUnit: 'มูลค่าเงินต้น / THB',
  activityNote: 'รวมเฉพาะรายการสำเร็จ · ไม่รวมค่าบริการ',
  activityCount: '/ {count} รายการ',
  buy: 'ร้านรับซื้อ',
  sell: 'ร้านขาย',

  // THB flow
  flowEyebrow: 'THB FLOW',
  flowTitle: 'รับเข้า − จ่ายออก',
  flowIn: 'รับเข้า',
  flowOut: 'จ่ายออก',
  flowNote1: 'ยอดชำระรวมค่าบริการ ไม่ใช่กำไร',
  flowNote2: 'ไม่ใช่ยอดเงินสดคงเหลือในลิ้นชัก',

  // Latest
  latestEyebrow: 'LATEST TRANSACTIONS',
  latestTitle: 'รายการล่าสุด',
  viewAll: 'ดูทั้งหมด →',
  emptyOverview: 'ไม่พบรายการในช่วงที่เลือก',

  // Status strip
  statusStripTitle: 'สถานะสาขาขณะนี้ · ไม่อิงช่วงวันที่',
  shiftOpen: 'กะเปิดอยู่',
  shiftClosed: 'ยังไม่เปิดกะ',
  statusError: 'ยังโหลดสถานะไม่ได้',
  openShiftHint: 'ตรวจนับเงินก่อนเริ่มทำรายการ',
  refreshHint: 'กดรีเฟรชเพื่อตรวจสอบข้อมูลล่าสุด',
  viewCashLink: 'ดูเงินสดและกะ →',
  pendingTotalLabel: 'รายการรอบันทึกทั้งหมด',
  pendingNote: 'ตรวจสถานะและเอกสารก่อนบันทึกซ้ำ',
  checkPendingBtn: 'ตรวจรายการ →',
  pendingItemCount: '{count} รายการ',
  openedByLabel: 'ผู้เปิดกะ {staff}',

  // CSV export
  exportCsv: 'ส่งออกรายการ CSV',
  noTransactionsToExport: 'ไม่มีรายการให้ส่งออก',
  outOfScope: 'สาขานี้อยู่นอกขอบเขตของบัญชีนี้',

  // Dialog
  dialogTitle: 'รายละเอียดรายการ',
} as const;

export const en: Record<keyof typeof th, string> = {
  menuLabel: 'Overview',
  eyebrow: 'DAILY OVERVIEW',
  title: 'Branch Overview',
  subtitle: 'Track shift status and transaction totals for the selected period',
  totalVolume: 'Total volume',
  totalVolumeNote: 'Principal value of completed transactions',
  buyCardLabel: 'Buy',
  buyCardNote: '{count} transactions · THB paid out',
  sellCardLabel: 'Sell',
  sellCardNote: '{count} transactions · THB received',
  completedTransactions: 'Completed transactions',
  completedUnit: 'transactions',
  completedNote: 'Cancelled {cancelled} · Pending {pending}',
  activityEyebrow: 'EXCHANGE ACTIVITY',
  activityTitle: 'Exchange ratio',
  activityUnit: 'Principal / THB',
  activityNote: 'Completed transactions only · excluding service fees',
  activityCount: '/ {count} transactions',
  buy: 'Buy',
  sell: 'Sell',
  flowEyebrow: 'THB FLOW',
  flowTitle: 'Received − Paid',
  flowIn: 'Received',
  flowOut: 'Paid out',
  flowNote1: 'Settlement including fees, not profit',
  flowNote2: 'Not the cash balance in the drawer',
  latestEyebrow: 'LATEST TRANSACTIONS',
  latestTitle: 'Latest transactions',
  viewAll: 'View all →',
  emptyOverview: 'No transactions in selected period',
  statusStripTitle: 'Current branch status · not bound to period',
  shiftOpen: 'Shift is open',
  shiftClosed: 'Shift not open',
  statusError: 'Unable to load status',
  openShiftHint: 'Count cash before starting transactions',
  refreshHint: 'Press refresh to check latest data',
  viewCashLink: 'View cash and shift →',
  pendingTotalLabel: 'Total pending transactions',
  pendingNote: 'Verify status and documents before recording again',
  checkPendingBtn: 'Check transactions →',
  pendingItemCount: '{count} transactions',
  openedByLabel: 'Opened by {staff}',
  exportCsv: 'Export CSV',
  noTransactionsToExport: 'No transactions to export',
  outOfScope: "This branch is outside this account's scope",
  dialogTitle: 'Transaction details',
};
