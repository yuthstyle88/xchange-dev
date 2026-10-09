// V2-payments (#470): payment settings i18n (W10, §7)

export const th: Record<string, string> = {
  // menu record
  label: 'วิธีชำระเงิน',
  eyebrow: 'PAYMENT SETTINGS',
  title: 'ตั้งค่าวิธีชำระเงิน',
  subtitle: 'เปิด–ปิดช่องทาง กำหนดข้อความแนะนำ และดูประวัติการตั้งค่า',

  // loading / error
  loading: 'กำลังโหลดวิธีชำระเงิน…',
  retry: 'โหลดอีกครั้ง',

  // intro and meta
  introEyebrow: 'PAYMENT CHANNELS',
  headingOf: 'ช่องทางของ',
  intro: 'ร้านขายเงินต่างประเทศ = ร้านรับเงินบาท · ร้านรับซื้อเงินต่างประเทศ = ร้านจ่ายเงินบาท',
  neverEdited: 'ใช้ค่าเริ่มต้นของระบบ · ยังไม่เคยแก้ไข',
  reload: 'โหลดล่าสุด',
  readOnly: 'ดูการตั้งค่าได้ · ให้เจ้าของระบบเป็นผู้แก้ไข',
  openCount: 'เปิด {open} / {total} ช่องทาง',
  editedMeta: 'แก้ไข {date} โดย {by}',
  branchNotice: 'กำลังแสดงการแก้ไขของสาขา {branch} (บันทึกหรือยกเลิกการแก้ไขก่อนเปลี่ยนสาขา)',

  // channels
  channelCash: 'เงินสด',
  channelCreditCard: 'บัตรเครดิต',
  channelAlipay: 'Alipay',
  channelPromptPay: 'พร้อมเพย์',
  channelBankTransfer: 'โอนธนาคาร',

  // a card
  enabled: 'เปิดใช้',
  sell: 'รับเงินบาทจากลูกค้า',
  sellNote: 'ร้านขายเงินต่างประเทศ',
  buy: 'จ่ายเงินบาทให้ลูกค้า',
  buyNote: 'ร้านรับซื้อเงินต่างประเทศ',
  receiveOnly: 'ช่องทางนี้ใช้รับเงินเท่านั้น',
  account: 'ชื่อบัญชี / จุดรับ–จ่าย',
  accountUnit: 'สำหรับพนักงาน',
  accountCash: 'เช่น เงินสดเคาน์เตอร์ 1',
  accountOther: 'เช่น บัญชีบริษัท / เครื่องรับบัตร 1',
  instruction: 'ข้อความแนะนำลูกค้า',
  instructionUnit: 'แสดงเมื่อเลือกช่องทางนี้',
  instructionPlaceholder: 'พิมพ์ภาษาไทยหรืออังกฤษสำหรับลูกค้า',
  evidenceCash: 'เงินสด: พนักงานตรวจนับก่อนยืนยัน',
  evidenceOther: 'ต้องมีเลขอ้างอิงและหลักฐานก่อนยืนยันรายการ',

  // save bar
  clean: 'บันทึกแล้ว / ยังไม่มีการแก้ไข',
  dirty: 'มีการแก้ไขที่ยังไม่บันทึก',
  saveNote: 'ใช้กับคำขอใหม่และการยืนยันรายการ · ไม่เปลี่ยนประวัติที่บันทึกสำเร็จแล้ว',
  discard: 'ยกเลิกการแก้ไข',
  save: 'บันทึกการตั้งค่า',
  saving: 'กำลังบันทึก…',
  saved: 'บันทึกการตั้งค่าสำเร็จ',
  footnote:
    'หากปิดช่องทางที่คิวค้างเลือกไว้ พนักงานต้องตรวจสอบและให้ลูกค้าเลือกใหม่ · การตั้งค่านี้ไม่ได้เชื่อมระบบตัดบัตรหรือยืนยันยอดธนาคารอัตโนมัติ',

  // history
  historySummary: 'ประวัติการตั้งค่า {count} รายการล่าสุด',
  historyHeaderDate: 'วันที่ / เวลา',
  historyHeaderActor: 'ผู้แก้ไข',
  historyHeaderChanges: 'ช่องทางที่เปลี่ยน',
  historyHeaderVersion: 'รุ่น',
  noHistory: 'ยังไม่มีประวัติการแก้ไข',
  unchanged: 'ยืนยันค่าชุดเดิม',

  // confirms
  confirmReload: 'โหลดล่าสุดและทิ้งการแก้ไขที่ยังไม่บันทึก?',
  confirmBranch: 'เปลี่ยนสาขาและทิ้งการแก้ไขวิธีชำระเงินที่ยังไม่บันทึก?',
};

export const en: Record<string, string> = {
  // menu record
  label: 'Payment Methods',
  eyebrow: 'PAYMENT SETTINGS',
  title: 'Payment Settings',
  subtitle: 'Enable or disable channels, set customer instructions, and view settings history',

  // loading / error
  loading: 'Loading payment methods…',
  retry: 'Retry loading',

  // intro and meta
  introEyebrow: 'PAYMENT CHANNELS',
  headingOf: 'Channels of',
  intro: 'Foreign currency selling shop = Shop receives THB · Foreign currency buying shop = Shop pays THB',
  neverEdited: 'System defaults · Never edited',
  reload: 'Reload latest',
  readOnly: 'View only · System owner can edit',
  openCount: 'Open {open} / {total} channels',
  editedMeta: 'Edited {date} by {by}',
  branchNotice: 'Showing edits for branch {branch} (save or discard before switching)',

  // channels
  channelCash: 'Cash',
  channelCreditCard: 'Credit Card',
  channelAlipay: 'Alipay',
  channelPromptPay: 'PromptPay',
  channelBankTransfer: 'Bank Transfer',

  // a card
  enabled: 'Enable',
  sell: 'Receive THB from customer',
  sellNote: 'Foreign currency selling shop',
  buy: 'Pay THB to customer',
  buyNote: 'Foreign currency buying shop',
  receiveOnly: 'This channel is receive-only',
  account: 'Account name / Point of service',
  accountUnit: 'For staff',
  accountCash: 'e.g. Cash Counter 1',
  accountOther: 'e.g. Company Account / Card Machine 1',
  instruction: 'Customer instructions',
  instructionUnit: 'Shown when selecting this channel',
  instructionPlaceholder: 'Type instructions in Thai or English for customers',
  evidenceCash: 'Cash: Staff counts before confirming',
  evidenceOther: 'Requires reference number and evidence before confirming',

  // save bar
  clean: 'Saved / No edits',
  dirty: 'Unsaved edits',
  saveNote: 'Applies to new requests and transaction confirmations · Does not change already completed history',
  discard: 'Discard edits',
  save: 'Save settings',
  saving: 'Saving…',
  saved: 'Settings saved successfully',
  footnote:
    'If closed while waiting in queue, staff must verify and have customer re-select · This setting does not connect to automated card or bank processing',

  // history
  historySummary: 'Payment settings history ({count} latest entries)',
  historyHeaderDate: 'Date / Time',
  historyHeaderActor: 'Editor',
  historyHeaderChanges: 'Channels changed',
  historyHeaderVersion: 'Version',
  noHistory: 'No edit history yet',
  unchanged: 'Same settings confirmed',

  // confirms
  confirmReload: 'Reload latest and discard unsaved edits?',
  confirmBranch: 'Change branch and discard unsaved payment method edits?',
};
