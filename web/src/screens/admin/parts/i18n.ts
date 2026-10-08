// V2-parts (#476): admin parts i18n table (W6, W10)

export const th = {
  // DataTable, admin-html:62
  emptyTransactions: 'ไม่พบรายการในช่วงที่เลือก',
  emptyTransactionsNote: 'เปลี่ยนช่วงวันที่หรือตัวกรอง แล้วลองอีกครั้ง',

  // transaction.ts, admin-html:56, :63
  buy: 'ร้านรับซื้อ',
  sell: 'ร้านขาย',
  methodCash: 'เงินสด',
  methodCreditCard: 'บัตรเครดิต',
  methodAlipay: 'Alipay',
  methodPromptPay: 'พร้อมเพย์',
  methodBankTransfer: 'โอนธนาคาร',
  methodOther: 'อื่น ๆ',
  statusCompleted: 'สำเร็จ',
  statusCancelled: 'ยกเลิก',
  statusPending: 'รอบันทึกเอกสาร',

  // TransactionTable, admin-html:63 with LIVE header of :100
  headerTxTime: 'รายการ / เวลา',
  headerCustomer: 'ลูกค้า',
  headerType: 'ประเภท',
  headerAmount: 'จำนวนเงิน',
  headerSettlement: 'ยอดชำระ THB',
  headerStatus: 'สถานะ',

  // TransactionDetail, admin-html:77 with LIVE override of :133
  detailId: 'เลขรายการ',
  detailDateTime: 'วันที่ / เวลา',
  detailRate: 'เรท',
  detailPrincipal: 'เงินต้น',
  detailFee: 'ค่าบริการ',
  detailSettlementBuy: 'ร้านจ่ายลูกค้า',
  detailSettlementSell: 'ร้านรับจากลูกค้า',
  detailBankFee: 'ค่าธรรมเนียมธนาคาร',
  detailMethod: 'ช่องทาง',
  detailStaff: 'พนักงาน',
  detailNotes: 'หมายเหตุ',
  detailRefRate: 'เรทอ้างอิง',
  detailRefValue: 'มูลค่าอ้างอิง',
  detailRefSpread: 'ส่วนต่างเทียบเรทอ้างอิง',
  detailCancelReason: 'เหตุผลยกเลิก',
  liveNote: 'ข้อมูลจาก เซิร์ฟเวอร์ · รายละเอียดนี้ไม่ใช่ใบเสร็จ',
  realTransactionVerifyRefund: 'รายการจริง: ตรวจสอบการคืนเงินกับผู้ดูแล',
  close: 'ปิด',

  // the slip section, slips.js:5, :18
  slipSectionTitle: 'สลิป / หลักฐานรับ–จ่าย',
  openSlip: 'เปิดสลิป ↗',
  download: 'ดาวน์โหลด',
  noSlipAttached: 'รายการนี้ไม่มีสลิปแนบ',

  // the receipt section, slips.js:30, :35, :38, :46
  receiptHeading: 'ใบเสร็จ',
  openReceipt: 'เปิดดู ใบเสร็จ',
  openReceiptImage: 'เปิดดู ภาพใบเสร็จ',
  receiptLoadFailed: 'โหลดใบเสร็จไม่ได้ กรุณาเข้าสู่ระบบแล้วลองใหม่',
  noReceiptYet: 'ยังไม่มีใบเสร็จที่บันทึกไว้สำหรับรายการนี้',
  loadingReceiptFor: 'กำลังโหลดใบเสร็จของ {id}…',
  savedDocumentsFor: 'เอกสารที่บันทึกไว้ · {id}',
  receiptTitleFor: 'ใบเสร็จ {id}',

  // Dialog, admin-html:22
  closeWindow: 'ปิดหน้าต่าง',

  // pending-write.ts, admin-html:120-123, :142
  serverSaved: 'เซิร์ฟเวอร์ ยืนยันการบันทึกแล้ว',
  notConfirmed: 'ยังไม่ได้รับการยืนยัน',
  notConfirmedNote: 'ยังไม่แสดงว่าบันทึกสำเร็จ การลองซ้ำใช้รหัสเดิมเพื่อป้องกันบันทึกซ้ำ กรุณาอย่าปิดหรือโหลดหน้านี้ใหม่',
  retry: 'ลองคำขอเดิมซ้ำ',
  inspect: 'ตรวจสถานะล่าสุด',
  foundSaved: 'ตรวจพบรายการบันทึกแล้ว',
  notFoundInServer: 'ยังไม่พบคำขอนี้ใน เซิร์ฟเวอร์',
  notFoundInServerText:
    'หากข้อความผิดพลาดระบุว่ากะหรือยอดเปลี่ยน ให้ยกเลิกคำขอนี้และตรวจข้อมูลล่าสุดก่อนทำใหม่ หากการเชื่อมต่อหมดเวลา ให้ลองคำขอเดิมซ้ำ',
  discard: 'ยกเลิกคำขอที่ยังไม่บันทึก',
  appliedToast: 'เซิร์ฟเวอร์ ยืนยันว่ารายการบันทึกแล้ว',
  cancelledToast: 'ยกเลิกคำขอที่ยังไม่บันทึกแล้ว',

  // NewTransactionButton, admin-html:100, :131
  newTransactionBtn: '＋ ทำรายการแลกเงิน',
  openShiftFirst: 'เปิดกะก่อนทำรายการแลกเงิน',
  newTransactionTitle: 'ทำรายการแลกเงินจริง',
  newTransactionSentence:
    'เปิดหน้าทำรายการเดิมเพื่อยืนยันลูกค้า เอกสาร และการรับ–จ่ายเงิน เมื่อ เซิร์ฟเวอร์ ยืนยันการบันทึก สต็อกในกะนี้จะปรับตามรายการ',
  openDeskLink: 'เปิดหน้าทำรายการ ↗',
  shiftStateReadError: 'อ่านสถานะกะไม่สำเร็จ',
} as const;

export type Key = keyof typeof th;

export const en: Record<Key, string> = {
  emptyTransactions: 'No transactions in selected period',
  emptyTransactionsNote: 'Change date range or filters and try again',

  buy: 'Buy',
  sell: 'Sell',
  methodCash: 'Cash',
  methodCreditCard: 'Credit Card',
  methodAlipay: 'Alipay',
  methodPromptPay: 'PromptPay',
  methodBankTransfer: 'Bank Transfer',
  methodOther: 'Other',
  statusCompleted: 'Completed',
  statusCancelled: 'Cancelled',
  statusPending: 'Pending',

  headerTxTime: 'Transaction / Time',
  headerCustomer: 'Customer',
  headerType: 'Type',
  headerAmount: 'Amount',
  headerSettlement: 'Settlement THB',
  headerStatus: 'Status',

  detailId: 'Transaction ID',
  detailDateTime: 'Date / Time',
  detailRate: 'Rate',
  detailPrincipal: 'Principal',
  detailFee: 'Fee',
  detailSettlementBuy: 'Paid to Customer',
  detailSettlementSell: 'Received from Customer',
  detailBankFee: 'Bank Fee',
  detailMethod: 'Payment Method',
  detailStaff: 'Staff',
  detailNotes: 'Notes',
  detailRefRate: 'Reference Rate',
  detailRefValue: 'Reference Value',
  detailRefSpread: 'Difference vs Ref Rate',
  detailCancelReason: 'Cancellation Reason',
  liveNote: 'Data from Server · This detail is not a receipt',
  realTransactionVerifyRefund: 'Live transaction: Verify refund with supervisor',
  close: 'Close',

  slipSectionTitle: 'Slip / Proof of Payment',
  openSlip: 'Open slip ↗',
  download: 'Download',
  noSlipAttached: 'No slip attached to this transaction',

  receiptHeading: 'Receipt',
  openReceipt: 'View Receipt',
  openReceiptImage: 'View Receipt Image',
  receiptLoadFailed: 'Failed to load receipt. Please sign in and try again',
  noReceiptYet: 'No receipt saved for this transaction yet',
  loadingReceiptFor: 'Loading receipt for {id}…',
  savedDocumentsFor: 'Saved documents · {id}',
  receiptTitleFor: 'Receipt {id}',

  closeWindow: 'Close window',

  serverSaved: 'Server confirmed the save',
  notConfirmed: 'Not Confirmed',
  notConfirmedNote:
    'Not yet confirmed as saved. Retry uses the same ID to prevent duplication. Please do not close or reload this page',
  retry: 'Retry Same Request',
  inspect: 'Check Latest Status',
  foundSaved: 'Found recorded transaction',
  notFoundInServer: 'Request not found on Server',
  notFoundInServerText:
    'If the error indicates shift or amount changed, cancel this request and check latest data before trying again. If connection timed out, retry the same request',
  discard: 'Cancel unrecorded request',
  appliedToast: 'Server confirmed transaction is recorded',
  cancelledToast: 'Unrecorded request cancelled',

  newTransactionBtn: '＋ New Exchange',
  openShiftFirst: 'Open a shift before starting exchange',
  newTransactionTitle: 'Live Exchange Transaction',
  newTransactionSentence:
    'Open the original transaction screen to verify customer, documents, and payment. Once the server confirms the save, this shift stock will adjust accordingly',
  openDeskLink: 'Open desk ↗',
  shiftStateReadError: 'Failed to read shift state',
};
