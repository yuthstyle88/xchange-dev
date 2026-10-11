// The booking entry: v2's Thai copy verbatim (design xchange-v3-wave3-desk.md §3.6 B1–B3, dh:178-191).
export const th = {
  loading: 'กำลังตรวจการจอง…',
  notConfirmed: 'การจองยังไม่ยืนยัน หมดอายุ หรือใช้ไปแล้ว',
  banner: 'จอง {id} · {name} · หมดอายุ {expires}',
  bannerNoExpiry: 'จอง {id} · {name}',
  identityLabel: 'ตรวจตัวตนลูกค้าตรงกับผู้จอง {name} แล้ว',
  confirmIdentity: 'กรุณายืนยันตัวตนว่าตรงกับผู้จองก่อนบันทึก',
  failed: 'ตรวจการจองไม่สำเร็จ เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่',
} as const;

export const en: Record<keyof typeof th, string> = {
  loading: 'Checking the booking…',
  notConfirmed: 'The booking is not confirmed, has expired or has been used',
  banner: 'Booking {id} · {name} · expires {expires}',
  bannerNoExpiry: 'Booking {id} · {name}',
  identityLabel: 'I checked the customer matches the booking holder {name}',
  confirmIdentity: 'Please confirm the customer matches the booking holder before saving',
  failed: 'Could not check the booking: no connection to the server. Please try again',
};
