// B06 ticket action texts (TH/EN parity). Filled by the actions ticket.
export const th = {
  call: 'เรียก',
  recall: 'เรียกซ้ำ',
  skip: 'ข้าม',
  start: 'เริ่มให้บริการ',
  finish: 'เสร็จสิ้น',
  cancel: 'ยกเลิก',
  cancelReason: 'เหตุผลที่ยกเลิก',
  cancelConfirm: 'ยืนยันยกเลิก',
  'error.cancelReasonRequired': 'กรุณาระบุเหตุผลที่ยกเลิก',
  'error.notYourTicket': 'คิวนี้ไม่ใช่เคาน์เตอร์ของคุณ',
  'error.counterPaused': 'เคาน์เตอร์พักอยู่ กรุณาเปิดรับคิวก่อน',
  'error.noOpenShift': 'ยังไม่ได้เปิดกะ กรุณาเปิดกะก่อน',
  'error.staleVersion': 'ข้อมูลคิวเปลี่ยนไปแล้ว กรุณาลองใหม่',
};

export const en: typeof th = {
  call: 'Call',
  recall: 'Recall',
  skip: 'Skip',
  start: 'Start',
  finish: 'Finish',
  cancel: 'Cancel',
  cancelReason: 'Cancel reason',
  cancelConfirm: 'Confirm cancel',
  'error.cancelReasonRequired': 'Please enter a cancellation reason',
  'error.notYourTicket': 'This ticket belongs to another counter',
  'error.counterPaused': 'Counter is paused; resume before calling',
  'error.noOpenShift': 'No open shift; open a shift first',
  'error.staleVersion': 'Ticket state changed; please try again',
};
