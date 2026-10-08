export const th = {
  // Menu and Headings
  label: 'เงินสดและกะ',
  headingEyebrow: 'CASH & SHIFTS',
  headingTitle: 'เงินสดและกะ',
  headingSubtitle: 'เปิดกะ รับ–จ่าย ตรวจนับ และติดตามประวัติในสาขาเดียวกัน',

  // Status Banner
  noOpenShift: 'ยังไม่มีกะเปิด',
  noOpenShiftSubtitle: 'ตรวจนับเงินสดและเงินต่างประเทศ แล้วกรอกยอดยกมาตอนเปิดกะ',
  openShiftBanner: 'กะกำลังเปิด · {id}',
  openShiftBannerSub: 'เปิด {when} · {staff} · ยอดกะไม่ขึ้นกับตัวกรองวันที่ของรายงาน',

  // Four Stat Cards
  statOpening: 'เงินสดตั้งต้น',
  statOpeningNote: 'ยอด THB ที่ตรวจนับตอนเปิดกะ',
  statIncoming: 'เงินสดรับเข้า',
  statIncomingNote: 'จากแลกเงินและรับเงินเข้าลิ้นชัก',
  statOutgoing: 'เงินสดจ่ายออก',
  statOutgoingNote: 'จ่ายให้ลูกค้าและจ่ายนอกการแลกเงิน',
  statRemaining: 'เงินสด THB คงเหลือ',
  statRemainingNote: 'ตั้งต้น + รับเข้า − จ่ายออก',

  // Reference Summary
  refTitle: 'ส่วนต่างเทียบเรทอ้างอิงในกะ',
  refNoRates: 'ยังไม่มีเรทอ้างอิง',
  refSummaryNoRates: 'ส่วนต่างเทียบเรทอ้างอิงในกะ: ยังไม่มีเรทอ้างอิง',
  refSummaryWithRates: 'ส่วนต่างเทียบเรทอ้างอิงในกะ: {spread} THB ({percent}%)',
  refSummaryCounts: '{withRef} รายการมีเรทอ้างอิง · {withoutRef} รายการยังไม่ระบุ · ไม่รวมค่าบริการและค่าใช้จ่าย',

  // Stock Card and Actions
  stockEyebrow: 'CASH & CURRENCY STOCK',
  stockTitle: 'เงินสดและสต็อกตามกะ',
  btnOpenShift: 'ตั้งเงินสดเริ่มต้น / เปิดกะ',
  btnViewExample: 'ดูตัวอย่าง 500,000',
  btnMoveStock: 'รับ–จ่ายเงินสด',
  btnCloseShift: 'ตรวจนับและปิดกะ',

  // Stock Table
  headerCurrency: 'สกุลเงิน',
  headerOpening: 'ยกมาตอนเปิดกะ',
  headerNet: 'เคลื่อนไหวสุทธิ',
  headerExpected: 'คงเหลือตามระบบ',
  stockNote1: 'เริ่มจากยอดเงินที่ตรวจนับจริง',
  stockNote2: 'รายการก่อนเปิดกะจะไม่ถูกนำมาหักซ้ำจากยอดยกมา',
  filterAll: 'แสดงทุกสกุล รวมยอดศูนย์',
  filterAllNote: 'ปกติแสดง THB สกุลที่ตั้งเรท และสกุลที่มียอดหรือการเคลื่อนไหว',

  // Footnote
  footnote: 'เงินสด THB ขยับเฉพาะรายการชำระเงินสด เงินต่างประเทศขยับทั้งรายการเงินสดและโอนธนาคาร ค่าธรรมเนียมธนาคารไม่หักจากลิ้นชักโดยอัตโนมัติ',

  // History Details
  historySummary: 'ประวัติรับ–จ่ายและกะที่ผ่านมา',
  historyMovementsTitle: 'ประวัติรับ–จ่ายนอกการแลกเงิน',
  headerDateTime: 'วัน / เวลา',
  headerShift: 'กะ',
  headerItem: 'รายการ',
  headerAmount: 'จำนวน',
  headerReason: 'เหตุผล',
  dirIn: 'รับเข้า',
  dirOut: 'จ่ายออก',
  emptyMovements: 'ยังไม่มีรายการรับ–จ่าย',

  historyShiftsTitle: 'ประวัติกะ',
  headerOpenedAt: 'เปิดเมื่อ',
  headerOpenedBy: 'ผู้เปิด',
  headerStatus: 'สถานะ',
  headerDifferenceClosed: 'ผลต่างตอนปิด',
  statusOpen: 'กำลังเปิด',
  statusClosed: 'ปิดแล้ว',
  diffMatched: 'ตรงตามระบบ',
  emptyShifts: 'ยังไม่มีประวัติกะ',
  loadOlder: 'โหลดรายการก่อนหน้า',

  // Open Shift Dialog
  openSearchLabel: 'ค้นหาและเพิ่มสกุลเงิน',
  openSearchPlaceholder: 'เช่น USD, ดอลลาร์, EUR',
  openSearchNotFound: 'ไม่พบสกุลเงิน หรือเพิ่มสกุลนี้แล้ว',
  remove: 'ลบ',
  cancel: 'ยกเลิก',
  confirmOpenShift: 'ยืนยันเปิดกะ',
  openNotePrefill: 'กรอกยอดเงินจริงเฉพาะสกุลที่มี ค้นหาแล้วกดเพิ่มสกุลเงินได้ สกุลที่ไม่ได้เพิ่มจะบันทึกเป็น 0 · เพิ่มสกุลที่มียอดจากกะก่อนให้แล้ว กรุณาตรวจนับยืนยันอีกครั้ง',
  openNoteClean: 'กรอกยอดเงินจริงเฉพาะสกุลที่มี ค้นหาแล้วกดเพิ่มสกุลเงินได้ สกุลที่ไม่ได้เพิ่มจะบันทึกเป็น 0',
  thbOpeningLabel: 'THB · เงินสดตั้งต้น',
  deleteCurrencyAria: 'ลบสกุลเงิน {code}',

  // Movement Dialog
  movementDialogTitle: 'บันทึกรับ–จ่ายเงินสด',
  movementDialogNote: 'สำหรับนำเงินเข้าลิ้นชัก นำฝาก หรือค่าใช้จ่ายเงินสด ไม่ต้องบันทึกซ้ำรายการแลกเงิน ระบบนี้บันทึกบัญชีเท่านั้น ไม่ได้โอนเงินจริง',
  movementTypeLabel: 'ประเภท',
  movementReasonLabel: 'เหตุผล / อ้างอิง',
  movementReasonPlaceholder: 'เช่น เงินเติมลิ้นชัก หรือ นำเงินฝากธนาคาร',
  saveMovement: 'บันทึกรับ–จ่าย',
  specifyReason: 'กรุณาระบุเหตุผล',

  // Close Shift Dialog
  closeThbLabel: 'THB · นับได้จริง',
  closeAsExpected: 'ตามระบบ',
  closeNoteLabel: 'เหตุผลยอดต่าง / หมายเหตุ',
  closeNotePlaceholder: 'ต้องระบุเมื่อมีเงินขาดหรือเกิน',
  backToCount: 'กลับไปตรวจนับ',
  confirmCloseShift: 'ยืนยันปิดกะ',

  // Shift Detail Dialog
  detailTitleClosed: 'ผลตรวจนับกะที่ปิดแล้ว',
  detailTitleOpen: 'รายละเอียดกะที่เปิด',
  headerCounted: 'นับจริง',
  headerDifference: 'ผลต่าง',
  close: 'ปิด',

  // Worked Example Dialog
  exampleTitle: 'ตัวอย่างเงินสดตั้งต้น 500,000 บาท',
  exampleSubtitle: 'ตัวอย่างคำนวณเท่านั้น ไม่บันทึกหรือเปลี่ยนยอดของคุณ',
  exampleCustomerSells: 'ลูกค้าขาย USD ให้ร้าน',
  exampleRefRate: 'เรทอ้างอิง',
  exampleShopRate: 'เรทที่ร้านจ่ายลูกค้า',
  exampleSpread: 'ส่วนต่างเทียบเรทอ้างอิง',
  exampleUsdStock: 'สต็อก USD หลังรับซื้อ',
  exampleExplanation: 'มุมมองร้านคือ “รับซื้อ USD” ร้านจ่าย THB ออก ส่วนต่าง 9,100 บาทเป็นมูลค่าเทียบเรทอ้างอิง ยังไม่ได้รับเป็นเงินสดเพิ่ม และสมมติว่าไม่มีค่าบริการ',
  understood: 'เข้าใจแล้ว',
};

export const en: Record<keyof typeof th, string> = {
  // Menu and Headings
  label: 'Cash & Shifts',
  headingEyebrow: 'CASH & SHIFTS',
  headingTitle: 'Cash & Shifts',
  headingSubtitle: 'Open shift, cash in/out, count balances, and track history for this branch',

  // Status Banner
  noOpenShift: 'No open shift',
  noOpenShiftSubtitle: 'Count cash and foreign currencies, then enter opening balances to open shift',
  openShiftBanner: 'Shift open · {id}',
  openShiftBannerSub: 'Opened {when} · {staff} · Shift figures are not tied to report date filters',

  // Four Stat Cards
  statOpening: 'Opening Cash',
  statOpeningNote: 'THB amount counted at shift open',
  statIncoming: 'Cash In',
  statIncomingNote: 'From exchanges and cash added to drawer',
  statOutgoing: 'Cash Out',
  statOutgoingNote: 'Paid to customers and expenses outside exchanges',
  statRemaining: 'Remaining THB Cash',
  statRemainingNote: 'Opening + In − Out',

  // Reference Summary
  refTitle: 'Reference rate spread in shift',
  refNoRates: 'No reference rates yet',
  refSummaryNoRates: 'Reference rate spread in shift: No reference rates yet',
  refSummaryWithRates: 'Reference rate spread in shift: {spread} THB ({percent}%)',
  refSummaryCounts: '{withRef} transactions with reference rate · {withoutRef} not specified · Excluding fees and charges',

  // Stock Card and Actions
  stockEyebrow: 'CASH & CURRENCY STOCK',
  stockTitle: 'Shift Cash & Stock',
  btnOpenShift: 'Set Opening Cash / Open Shift',
  btnViewExample: 'View 500,000 Example',
  btnMoveStock: 'Cash In/Out',
  btnCloseShift: 'Count & Close Shift',

  // Stock Table
  headerCurrency: 'Currency',
  headerOpening: 'Opening Balance',
  headerNet: 'Net Movement',
  headerExpected: 'System Expected',
  stockNote1: 'Starting from actual counted balance',
  stockNote2: 'Transactions before shift open are not deducted again from opening balance',
  filterAll: 'Show all currencies including zero balances',
  filterAllNote: 'Normally shows THB, rated currencies, and currencies with balance or movement',

  // Footnote
  footnote: 'THB cash moves only on cash transactions. Foreign currencies move on both cash and transfer. Bank fees are not automatically deducted from drawer.',

  // History Details
  historySummary: 'History of Cash Movements & Past Shifts',
  historyMovementsTitle: 'Non-Exchange Cash Movements History',
  headerDateTime: 'Date / Time',
  headerShift: 'Shift',
  headerItem: 'Item',
  headerAmount: 'Amount',
  headerReason: 'Reason',
  dirIn: 'Cash In',
  dirOut: 'Cash Out',
  emptyMovements: 'No cash movements yet',

  historyShiftsTitle: 'Shift History',
  headerOpenedAt: 'Opened At',
  headerOpenedBy: 'Opened By',
  headerStatus: 'Status',
  headerDifferenceClosed: 'Closing Difference',
  statusOpen: 'Open',
  statusClosed: 'Closed',
  diffMatched: 'Matches System',
  emptyShifts: 'No shift history yet',
  loadOlder: 'Load older',

  // Open Shift Dialog
  openSearchLabel: 'Search and add currency',
  openSearchPlaceholder: 'e.g. USD, Dollar, EUR',
  openSearchNotFound: 'Currency not found or already added',
  remove: 'Remove',
  cancel: 'Cancel',
  confirmOpenShift: 'Confirm Open Shift',
  openNotePrefill: 'Enter actual amounts for available currencies. Search and click to add. Currencies not added will be recorded as 0. Currencies with balances from previous shift have been prefilled; please verify by counting.',
  openNoteClean: 'Enter actual amounts for available currencies. Search and click to add. Currencies not added will be recorded as 0.',
  thbOpeningLabel: 'THB · Opening cash',
  deleteCurrencyAria: 'Remove currency {code}',

  // Movement Dialog
  movementDialogTitle: 'Record Cash In/Out',
  movementDialogNote: 'For adding cash to drawer, deposits, or cash expenses. Do not duplicate exchange transactions. This system records accounts only, no real money transfer.',
  movementTypeLabel: 'Type',
  movementReasonLabel: 'Reason / Reference',
  movementReasonPlaceholder: 'e.g. Cash added to drawer or bank deposit',
  saveMovement: 'Save Cash Movement',
  specifyReason: 'Please specify a reason',

  // Close Shift Dialog
  closeThbLabel: 'THB · Actual Count',
  closeAsExpected: 'Expected',
  closeNoteLabel: 'Difference Reason / Notes',
  closeNotePlaceholder: 'Required when there is a shortage or excess',
  backToCount: 'Back to Count',
  confirmCloseShift: 'Confirm Close Shift',

  // Shift Detail Dialog
  detailTitleClosed: 'Closed Shift Count Results',
  detailTitleOpen: 'Open Shift Details',
  headerCounted: 'Counted',
  headerDifference: 'Difference',
  close: 'Close',

  // Worked Example Dialog
  exampleTitle: '500,000 THB Opening Cash Example',
  exampleSubtitle: 'Calculation example only; does not record or alter your balances',
  exampleCustomerSells: 'Customer sells USD to shop',
  exampleRefRate: 'Reference rate',
  exampleShopRate: 'Rate shop pays customer',
  exampleSpread: 'Spread vs reference rate',
  exampleUsdStock: 'USD stock after purchase',
  exampleExplanation: 'From the shop\'s perspective of "buying USD", the shop pays out THB. The 9,100 THB difference is value compared to the reference rate, has not been received as additional cash, and assumes no service fees.',
  understood: 'Understood',
};
