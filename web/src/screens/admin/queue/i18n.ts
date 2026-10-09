// V2-queue (#471): admin customer-queue bar texts (W6, W10)

export const th = {
  barAria: 'คิวลูกค้าและผู้ทำรายการ',
  titleInitial: 'คิวลูกค้าหน้าร้าน',
  checkingInitial: 'กำลังตรวจคิว…',
  checkingBranch: 'กำลังตรวจคิวสาขา…',
  openQueue: 'เปิดคิว / รับเรื่อง →',
  closeDialog: 'ปิด ×',
  viewQueue: 'ดูคิว',
  closeToast: 'ปิด',
  offline: 'เชื่อมต่อคิวไม่ได้ · กำลังลองใหม่',
  noData: 'ยังไม่มีข้อมูล',
  status: 'รอรับเรื่อง {waiting} · กำลังทำรายการ {working} · {branch}',
  workerLine: '{staff} กำลังทำรายการ {queue} · เริ่ม {time}',
  toastNotice: 'มีคิวลูกค้ารอ {count} รายการ · {branch}',
  titleWithBranch: 'คิวลูกค้า · {branch}',
};

export const en: Record<keyof typeof th, string> = {
  barAria: 'Customer queue and handlers',
  titleInitial: 'Front-desk queue',
  checkingInitial: 'Checking queue…',
  checkingBranch: 'Checking branch queue…',
  openQueue: 'Open queue / take request →',
  closeDialog: 'Close ×',
  viewQueue: 'View queue',
  closeToast: 'Close',
  offline: 'Cannot connect to queue · Retrying',
  noData: 'No data',
  status: 'Waiting {waiting} · Serving {working} · {branch}',
  workerLine: '{staff} serving {queue} · started {time}',
  toastNotice: '{count} customers waiting · {branch}',
  titleWithBranch: 'Customer queue · {branch}',
};
