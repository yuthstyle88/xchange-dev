// V2-bookings (#463): bookings menu i18n table (W6, W10)

export const th = {
  // Navigation and headings
  navLabel: 'การจอง',
  headingEyebrow: 'ONLINE BOOKINGS',
  headingSubtitle: 'ตรวจการจอง ล็อกเรท และส่งต่อไปทำรายการหน้าร้าน',

  // Toolbar and search
  openPublicWeb: 'เปิดเว็บจองสำหรับลูกค้า ↗',
  refreshBookings: 'โหลดการจองล่าสุด',
  bookingSettings: 'ตั้งค่าการจอง',
  searchBookings: 'ค้นหาการจอง',
  searchPlaceholder: 'เลขจอง / ชื่อ / เบอร์โทร',

  // Loading and empty
  loading: 'กำลังโหลดการจอง…',
  emptySearch: 'ยังไม่มีการจองที่ตรงกับคำค้นในสาขานี้',
  noCurrencies: 'ไม่มีสกุลเงินที่เปิดรับจอง',

  // Info line
  infoOpen: 'เปิดรับจองเมื่อสาขาเปิดกะและตั้งเรทแล้ว',
  infoClosed: 'ปิดรับจองใหม่',
  infoPrefix: 'ลูกค้าใหม่ ',
  infoMinutes: ' นาที · ',
  infoMembers: ' · Member ที่ยืนยันตัวตน ',
  infoPersons: ' คน',

  // Table headers
  thBookingCustomer: 'เลขจอง / ลูกค้า',
  thItem: 'รายการ',
  thRate: '~เรท',
  thExpiry: 'หมดอายุ',
  thStatus: 'สถานะ',
  thActions: 'จัดการ',

  // Row and actions
  pendingStaff: 'รอพนักงานกำหนด',
  confirmRateAndTime: 'ยืนยันเรทและเวลา',
  deskHandOff: 'ทำรายการหน้าร้าน ↗',
  cancel: 'ยกเลิก',
  memberPill: 'Member',
  typeBuy: 'ร้านรับซื้อ',
  typeSell: 'ร้านขาย',

  // Statuses
  statusPending: 'Member · รอยืนยัน',
  statusConfirmed: 'ล็อกเรทแล้ว',
  statusDone: 'ทำรายการแล้ว',
  statusExpired: 'หมดอายุ',
  statusCancelled: 'ยกเลิก',

  // Confirm dialog
  dialogConfirmTitle: 'ยืนยันการจอง Member',
  rateNotSet: 'ยังไม่ตั้งเรท',
  expiryField: 'วันเวลาหมดอายุ (เวลาของเครื่องนี้)',
  back: 'กลับ',
  confirmLockRate: 'ยืนยันล็อกเรท',
  rateToLock: 'เรทที่จะล็อก: ',
  principal: ' · ยอดเงินต้น ',
  dash: '—',
  thb: ' THB',

  // Settings dialog
  settingsMinutes: 'ล็อกเรทลูกค้าใหม่ (นาที)',
  settingsOpenWhenShift: 'เปิดรับจองเมื่อมีกะเปิดและเรทพร้อม',
  settingsMemberNote: 'Member ให้พนักงานกำหนดเวลาหมดอายุแต่ละรายการ',
  save: 'บันทึก',
  settingsBranchPrefix: 'ตั้งค่าการจองสาขา ',

  // Cancel dialog
  dialogCancelTitle: 'ยกเลิกการจอง',
  confirmCancel: 'ยืนยันยกเลิก',
  cancelConfirmPrefix: 'ยกเลิก ',
  cancelConfirmSuffix: ' และยุติการล็อกเรทของการจองนี้?',

  // Create form
  fieldName: 'ชื่อ',
  fieldCustomerId: 'รหัสลูกค้า',
  fieldPhone: 'เบอร์ติดต่อ',
  fieldType: 'ประเภท',
  fieldAmount: 'จำนวนเงิน',
  fieldRate: 'เรท',
  fieldCurrency: 'สกุลเงิน',
  createButton: 'สร้างการจอง',
  createButtonWithPlus: '＋ สร้างการจอง',

  // Validation messages
  errFillIn: 'กรอกข้อมูลให้ครบและไม่เกินกำหนด',
  errBadPhone: 'เบอร์ติดต่อไม่ถูกต้อง',
  errBadAmountOrRate: 'จำนวนเงินหรือเรทไม่ถูกต้อง',

  // Paging
  loadMore: 'โหลดเพิ่มเติม',
} as const;

export const en: Record<keyof typeof th, string> = {
  // Navigation and headings
  navLabel: 'Bookings',
  headingEyebrow: 'ONLINE BOOKINGS',
  headingSubtitle: 'Check bookings, lock rates, and send to counter',

  // Toolbar and search
  openPublicWeb: 'Open customer booking website ↗',
  refreshBookings: 'Reload latest bookings',
  bookingSettings: 'Booking settings',
  searchBookings: 'Search bookings',
  searchPlaceholder: 'Booking ID / Name / Phone',

  // Loading and empty
  loading: 'Loading bookings…',
  emptySearch: 'No bookings matching search in this branch',
  noCurrencies: 'No currencies available',

  // Info line
  infoOpen: 'Open for bookings when branch shift is open and rates are set',
  infoClosed: 'Closed for new bookings',
  infoPrefix: 'New customers ',
  infoMinutes: ' min · ',
  infoMembers: ' · Verified members ',
  infoPersons: ' members',

  // Table headers
  thBookingCustomer: 'Booking ID / Customer',
  thItem: 'Item',
  thRate: '~Rate',
  thExpiry: 'Expires',
  thStatus: 'Status',
  thActions: 'Actions',

  // Row and actions
  pendingStaff: 'Pending staff confirmation',
  confirmRateAndTime: 'Confirm rate and time',
  deskHandOff: 'Process at counter ↗',
  cancel: 'Cancel',
  memberPill: 'Member',
  typeBuy: 'Buy',
  typeSell: 'Sell',

  // Statuses
  statusPending: 'Member · Pending',
  statusConfirmed: 'Rate locked',
  statusDone: 'Completed',
  statusExpired: 'Expired',
  statusCancelled: 'Cancelled',

  // Confirm dialog
  dialogConfirmTitle: 'Confirm Member Booking',
  rateNotSet: 'Rate not set',
  expiryField: 'Expiry date/time (this device time)',
  back: 'Back',
  confirmLockRate: 'Confirm lock rate',
  rateToLock: 'Rate to lock: ',
  principal: ' · Principal ',
  dash: '—',
  thb: ' THB',

  // Settings dialog
  settingsMinutes: 'Lock rate for new customers (minutes)',
  settingsOpenWhenShift: 'Open for bookings when shift is open and rates are ready',
  settingsMemberNote: 'Staff determines expiry time for each member booking',
  save: 'Save',
  settingsBranchPrefix: 'Booking settings for branch ',

  // Cancel dialog
  dialogCancelTitle: 'Cancel Booking',
  confirmCancel: 'Confirm Cancel',
  cancelConfirmPrefix: 'Cancel ',
  cancelConfirmSuffix: ' and terminate rate lock for this booking?',

  // Create form
  fieldName: 'Name',
  fieldCustomerId: 'Customer ID',
  fieldPhone: 'Phone',
  fieldType: 'Type',
  fieldAmount: 'Amount',
  fieldRate: 'Rate',
  fieldCurrency: 'Currency',
  createButton: 'Create booking',
  createButtonWithPlus: '＋ Create booking',

  // Validation messages
  errFillIn: 'Fill in all required fields within limit',
  errBadPhone: 'Invalid phone number',
  errBadAmountOrRate: 'Invalid amount or rate',

  // Paging
  loadMore: 'Load more',
};
