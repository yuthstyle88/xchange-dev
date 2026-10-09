export const th = {
  // Menu
  report: 'รายงาน',
  eyebrow: 'DAILY REPORT',
  subtitle: 'แยกดูช่องทางชำระเงิน สกุลเงิน และยอดของพนักงาน',

  // Tabs
  tablistAria: 'ประเภทรายงาน',
  tabPayment: 'ช่องทางชำระเงิน',
  tabCurrencies: 'สกุลเงินและส่วนต่าง',
  tabEmployees: 'แยกพนักงาน',

  // Stat cards
  statReceivedLabel: 'รับเงินบาทรวม',
  statReceivedNote: 'ยอดชำระจากรายการร้านขาย',
  statPaidLabel: 'จ่ายเงินบาทรวม',
  statPaidNote: 'ยอดชำระจากรายการร้านรับซื้อ',
  statFeeLabel: 'ค่าบริการลูกค้า',
  statFeeNote: 'รวมอยู่ในยอดชำระแล้ว',
  statBankFeeLabel: 'ค่าธรรมเนียมธนาคาร',
  statBankFeeNote: 'แยกจากยอดชำระลูกค้า',

  // Settlement / Payment
  settlementEyebrow: 'SETTLEMENT SUMMARY',
  settlementHeading: 'สรุปตามช่องทางชำระเงิน',
  colChannel: 'ช่องทาง',
  colCount: 'จำนวนรายการ',
  colReceived: 'รับเข้า',
  colPaid: 'จ่ายออก',
  colNet: 'รับเข้า − จ่ายออก',
  footnote: 'รวมเฉพาะรายการสำเร็จตามช่วงวันที่และสาขาที่เลือก · ยอดรับ–จ่ายและส่วนต่างเรทไม่ใช่กำไรสุทธิ',

  // Currencies
  currenciesEyebrow: 'CURRENCY BREAKDOWN',
  currenciesHeading: 'สรุปตามสกุลเงิน',
  colCurrency: 'สกุลเงิน',
  colBuyUnits: 'ร้านรับซื้อ (หน่วยเงิน)',
  colSellUnits: 'ร้านขาย (หน่วยเงิน)',
  colPrincipalThb: 'เงินต้นรวม THB',
  colFeeThb: 'ค่าบริการ THB',

  // Reference comparison
  noReferenceRate: 'ยังไม่มีเรทอ้างอิง',
  referenceSpreadPrefix: 'ส่วนต่างเทียบเรทอ้างอิงในช่วงวันที่: ',
  referenceNote: 'ไม่รวมค่าบริการและค่าใช้จ่าย ส่วนต่างนี้ไม่ถูกบวกเข้าลิ้นชัก และสำหรับเงินที่รับซื้อไว้ยังไม่ใช่กำไรจากการขายต่อ',
  referenceCounts: '{referenced} รายการมีเรทอ้างอิง · {unreferenced} รายการยังไม่ระบุ · {note}',

  // Employees summary
  employeeSummaryHeading: 'สรุปยอดแยกพนักงาน',
  downloadCsv: 'ดาวน์โหลด CSV',
  employeeSummaryNote: 'อิงสาขาและช่วงวันที่ด้านบน · รับเข้า/จ่ายออกจากมุมร้าน หน่วย THB · รวมเฉพาะรายการสำเร็จ · ค่าบริการรวมในยอดรับ–จ่ายแล้ว ค่าธรรมเนียมธนาคารแสดงแยก · ยอดสุทธิไม่ใช่กำไร',
  employeeLabel: 'พนักงาน',
  allStaff: 'ทุกคน',
  searchEmployee: 'ค้นหาพนักงาน',
  staffSearchPlaceholder: 'ชื่อบัญชี / รหัสพนักงาน',
  totalsLine: 'รวม {people} คน · สำเร็จ {completed} รายการ · รับเข้า {received} · จ่ายออก {paid} · สุทธิ {net} THB',

  colAll: 'ทั้งหมด',
  colCompleted: 'สำเร็จ',
  colPending: 'ค้าง',
  colCancelled: 'ยกเลิก',
  colReceivedThb: 'รับเข้า THB',
  colPaidThb: 'จ่ายออก THB',
  colNetThb: 'สุทธิ THB',
  colServiceFee: 'ค่าบริการ',
  colBankFee: 'ค่าธรรมเนียมธนาคาร',

  // Employee details
  employeeSectionSummary: '{staff} · {count} รายการ — ดูแยกช่องทางและทุกรายการ',
  methodsHeading: 'รับ–จ่ายเงินบาท แยกช่องทาง',
  employeeCurrenciesHeading: 'เงินต่างประเทศที่รับ–จ่าย',
  colUnitsReceived: 'รับเข้า (หน่วยเงิน)',
  colUnitsPaid: 'จ่ายออก (หน่วยเงิน)',
  transactionsHeading: 'รายการทั้งหมดของพนักงาน',

  // Employee 7 columns
  colTxTime: 'เลขรายการ / เวลา',
  colStatus: 'สถานะ',
  colType: 'ประเภท',
  colAmount: 'จำนวนเงิน',

  // Employee CSV headers
  csvTxId: 'เลขรายการ',
  csvDate: 'วันที่',
  csvStatus: 'สถานะ',
  csvType: 'ประเภท',
  csvAmount: 'จำนวน',
  csvFeeThb: 'ค่าบริการ THB',
  csvBankFeeThb: 'ค่าธรรมเนียมธนาคาร THB',

  // Export
  exportCsv: 'ส่งออกรายการ CSV',
  noTransactionsToExport: 'ไม่มีรายการให้ส่งออก',
};

export const en = {
  // Menu
  report: 'Daily Report',
  eyebrow: 'DAILY REPORT',
  subtitle: 'Breakdown of payment channels, currencies and employee totals',

  // Tabs
  tablistAria: 'Report Type',
  tabPayment: 'Payment Channels',
  tabCurrencies: 'Currencies & Spreads',
  tabEmployees: 'By Employee',

  // Stat cards
  statReceivedLabel: 'Total THB Received',
  statReceivedNote: 'Settlement from sell transactions',
  statPaidLabel: 'Total THB Paid',
  statPaidNote: 'Settlement from buy transactions',
  statFeeLabel: 'Customer Service Fees',
  statFeeNote: 'Included in settlement amount',
  statBankFeeLabel: 'Bank Fees',
  statBankFeeNote: 'Separate from customer settlement',

  // Settlement / Payment
  settlementEyebrow: 'SETTLEMENT SUMMARY',
  settlementHeading: 'Summary by Payment Channel',
  colChannel: 'Channel',
  colCount: 'Transactions',
  colReceived: 'Inflow',
  colPaid: 'Outflow',
  colNet: 'Inflow − Outflow',
  footnote: 'Completed transactions only for selected branch and date range · Inflow/outflow and rate spreads are not net profit',

  // Currencies
  currenciesEyebrow: 'CURRENCY BREAKDOWN',
  currenciesHeading: 'Summary by Currency',
  colCurrency: 'Currency',
  colBuyUnits: 'Purchased (Units)',
  colSellUnits: 'Sold (Units)',
  colPrincipalThb: 'Principal THB',
  colFeeThb: 'Fees THB',

  // Reference comparison
  noReferenceRate: 'No reference rate yet',
  referenceSpreadPrefix: 'Spread vs reference rate in period: ',
  referenceNote: 'Excludes fees and expenses. This spread is not added to the till and un-resold purchases are not yet realized profit',
  referenceCounts: '{referenced} with reference rate · {unreferenced} unspecified · {note}',

  // Employees summary
  employeeSummaryHeading: 'Employee Breakdown',
  downloadCsv: 'Download CSV',
  employeeSummaryNote: 'Based on branch and dates above · Shop perspective in THB · Completed transactions only · Fees included in in/out, bank fees separate · Net is not profit',
  employeeLabel: 'Employee',
  allStaff: 'Everyone',
  searchEmployee: 'Search Employee',
  staffSearchPlaceholder: 'Username / Staff ID',
  totalsLine: 'Total {people} people · {completed} completed · Received {received} · Paid {paid} · Net {net} THB',

  colAll: 'Total',
  colCompleted: 'Completed',
  colPending: 'Pending',
  colCancelled: 'Cancelled',
  colReceivedThb: 'Inflow THB',
  colPaidThb: 'Outflow THB',
  colNetThb: 'Net THB',
  colServiceFee: 'Fees',
  colBankFee: 'Bank Fee',

  // Employee details
  employeeSectionSummary: '{staff} · {count} transactions — breakdown and all transactions',
  methodsHeading: 'THB Cashflow by Channel',
  employeeCurrenciesHeading: 'Foreign Currencies Exchanged',
  colUnitsReceived: 'Inflow (Units)',
  colUnitsPaid: 'Outflow (Units)',
  transactionsHeading: 'All Employee Transactions',

  // Employee 7 columns
  colTxTime: 'Tx ID / Time',
  colStatus: 'Status',
  colType: 'Type',
  colAmount: 'Amount',

  // Employee CSV headers
  csvTxId: 'Tx ID',
  csvDate: 'Date',
  csvStatus: 'Status',
  csvType: 'Type',
  csvAmount: 'Amount',
  csvFeeThb: 'Fee THB',
  csvBankFeeThb: 'Bank Fee THB',

  // Export
  exportCsv: 'Export CSV',
  noTransactionsToExport: 'No transactions to export',
};
