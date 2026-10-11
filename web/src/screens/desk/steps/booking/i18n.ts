// Step 2: v2's Thai copy verbatim (design xchange-v3-wave3-desk.md §3.6b row "Step 2 booking", dh:116).
export const th = {
  label: 'การจอง',
  heading: 'ข้อมูลการจอง',
  name: 'ชื่อลูกค้า',
  staff: 'รหัสพนักงาน',
  staffTitle: 'รหัสจากบัญชีที่เข้าสู่ระบบ',
  valueDate: 'วันที่มีผล',
  back: '← แก้ไขข้อมูลลูกค้า',
  submit: 'ยืนยันการจอง →',
  needNameStaff: 'กรุณาระบุชื่อลูกค้าและรหัสพนักงาน',
} as const;

export const en: Record<keyof typeof th, string> = {
  label: 'Booking',
  heading: 'Booking details',
  name: 'Customer name',
  staff: 'Staff code',
  staffTitle: 'The code of the signed-in account',
  valueDate: 'Value date',
  back: '← Edit customer details',
  submit: 'Confirm booking →',
  needNameStaff: 'Please enter the customer name and the staff code',
};
