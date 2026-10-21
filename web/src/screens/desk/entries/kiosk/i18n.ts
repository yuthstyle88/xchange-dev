// B03 kiosk entry texts (TH/EN parity). Filled by the kiosk entry ticket.
export const th = {
  banner: 'ข้อมูลจากคิว {queue}',
  cancelRequest: 'ยกเลิกคำขอ',
  identityLabel: 'ลูกค้าหน้าเคาน์เตอร์คือผู้ทำรายการที่ตู้คีออสก์',
  identityRequired: 'กรุณายืนยันตัวตนลูกค้าจากตู้คีออสก์ก่อนทำรายการ',
};

export const en: typeof th = {
  banner: 'From queue {queue}',
  cancelRequest: 'Cancel request',
  identityLabel: 'The customer at the counter is the one who requested at the kiosk',
  identityRequired: 'Please confirm that the customer matches the kiosk request',
};
