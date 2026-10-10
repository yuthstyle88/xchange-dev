// V2-shell (#460): admin shell text table (ticket §7, §9, W6, W10)

export const th = {
  groupDesk: 'งานหน้าร้าน',
  groupMoney: 'เงินและรายงาน',
  groupData: 'ข้อมูลและทีม',
  groupTools: 'เครื่องมือ',

  linkQueue: 'คิวลูกค้า',
  linkKiosk: 'เปิดหน้าลูกค้า ↗',
  linkDevices: 'งานอุปกรณ์',
  linkGuide: 'คู่มือพนักงาน ↗',
  linkReceiptSample: 'ตัวอย่างใบเสร็จ',

  searchPlaceholder: 'ค้นหาเมนู…',
  noMenu: 'ไม่พบเมนู ลองใช้คำอื่น',
  menuButton: '☰ เมนู',
  closeMenu: 'ปิดเมนู',

  manageBranches: 'จัดการสาขา',
  addBranch: '＋ สาขา',
  refresh: 'รีเฟรช',

  today: 'วันนี้',
  allDates: 'ทั้งหมด',
  dateError: 'วันที่เริ่มต้นต้องไม่เกินวันที่สิ้นสุด',

  connection: 'ข้อมูลและการเชื่อมต่อ ↗',
  connectionTitle: 'บัญชีออนไลน์',
  connection1:
    'บัญชีนี้เก็บข้อมูลบนเซิร์ฟเวอร์ใหม่ เปิดกะและตั้งเรทก่อนเริ่มใช้งาน ข้อมูลเดิมใน Google ยังไม่ได้ย้ายเข้ามา',
  connection2:
    'รายการทุกสาขาและไฟล์แนบใช้ร่วมกันได้เมื่อเปิดจากบัญชีเจ้าของเว็บนี้',

  liveOnline: 'เซิร์ฟเวอร์ · ข้อมูลจริง',
  liveOffline: 'เซิร์ฟเวอร์ · ยังไม่พร้อมใช้งาน',
  lastSync: 'อัปเดตล่าสุด ',

  addTitle: 'เพิ่มสาขา',
  addNote:
    'แต่ละสาขามีเงินสด สต็อก กะ รายการ และเรทของตัวเอง สาขาใหม่เริ่มโดยไม่มีรายการหรือกะ',
  addPlaceholder: 'เช่น XC-C',
  addSubmit: 'สร้างสาขา',
  addError: 'กรุณาระบุชื่อสาขาที่ไม่ซ้ำ',
  pageTitle: 'XCHANGE · แอดมินหน้าร้าน',
};

export const en = {
  groupDesk: 'Counter Operations',
  groupMoney: 'Money & Reports',
  groupData: 'Data & Team',
  groupTools: 'Tools',

  linkQueue: 'Customer Queue',
  linkKiosk: 'Customer Screen ↗',
  linkDevices: 'Devices',
  linkGuide: 'Staff Guide ↗',
  linkReceiptSample: 'Receipt Sample',

  searchPlaceholder: 'Search menus…',
  noMenu: 'No menus found. Try another search.',
  menuButton: '☰ Menu',
  closeMenu: 'Close menu',

  manageBranches: 'Manage Branches',
  addBranch: '＋ Branch',
  refresh: 'Refresh',

  today: 'Today',
  allDates: 'All',
  dateError: 'Start date cannot be after end date',

  connection: 'Data & Connection ↗',
  connectionTitle: 'Online Account',
  connection1:
    'This account stores data on the new server. Open a shift and set rates before use. Historical data in Google has not been migrated.',
  connection2:
    'Transactions from all branches and attachments are shared when accessed from the owner account.',

  liveOnline: 'Server · Live Data',
  liveOffline: 'Server · Not Ready',
  lastSync: 'Last update: ',

  addTitle: 'Add Branch',
  addNote:
    'Each branch has its own cash, stock, shifts, transactions, and rates. New branches start with no transactions or shifts.',
  addPlaceholder: 'e.g. XC-C',
  addSubmit: 'Create Branch',
  addError: 'Please specify a unique branch name',
  pageTitle: 'XCHANGE · Store Admin',
};
