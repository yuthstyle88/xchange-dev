// B03 phone verification texts (TH/EN parity). Filled by the phone verification ticket.
export const th = {
  phone: 'เบอร์โทรศัพท์',
  sendCode: 'ส่งรหัส OTP',
  code: 'รหัส OTP',
  confirmCode: 'ยืนยันรหัส',
  verified: 'ยืนยันเบอร์โทรศัพท์แล้ว',
  'error.wrongCode': 'รหัส OTP ไม่ถูกต้อง',
  'error.codeExpired': 'รหัส OTP หมดอายุ กรุณาขอรหัสใหม่',
  'error.tooManyAttempts': 'กรอกรหัสผิดเกินจำนวนครั้งที่กำหนด กรุณาขอรหัสใหม่',
  'error.rateLimited': 'ขอรหัสถี่เกินไป กรุณารอสักครู่แล้วลองใหม่',
  'error.phoneInUse': 'เบอร์โทรศัพท์นี้ผูกกับลูกค้ารายอื่นแล้ว',
  'error.smsUnavailable': 'ระบบส่ง SMS ขัดข้องชั่วคราว กรุณาลองใหม่',
} as const;

export const en: Record<keyof typeof th, string> = {
  phone: 'Phone',
  sendCode: 'Send OTP',
  code: 'OTP code',
  confirmCode: 'Confirm code',
  verified: 'Phone verified',
  'error.wrongCode': 'Incorrect OTP code',
  'error.codeExpired': 'OTP code has expired. Please request a new code',
  'error.tooManyAttempts': 'Too many incorrect attempts. Please request a new code',
  'error.rateLimited': 'Too many requests. Please wait and try again',
  'error.phoneInUse': 'This phone number is already linked to another customer',
  'error.smsUnavailable': 'SMS service is temporarily unavailable. Please try again',
};
