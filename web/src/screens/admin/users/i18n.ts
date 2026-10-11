export const th = {
  label: 'ผู้ใช้งานและสิทธิ์',
  eyebrow: 'ORGANIZATION & USERS',
  title: 'ผู้ใช้งานและสิทธิ์',
  subtitle: 'จัดการบัญชีผู้ใช้งาน บทบาท สิทธิ์ และการเข้าถึงระบบ',

  heading: 'รายชื่อผู้ใช้งาน',
  intro: 'ผู้ดูแลระบบ (ADMIN) สามารถสร้างบัญชี มอบหมายสิทธิ์ ยกเลิกสิทธิ์ หรือปิดการใช้งานบัญชี',
  createButton: 'สร้างผู้ใช้ใหม่',
  refresh: 'รีเฟรช',
  noUsers: 'ไม่พบผู้ใช้งาน',
  loading: 'กำลังโหลด...',

  colName: 'ชื่อผู้ใช้ / รหัส',
  colRoles: 'บทบาทและสาขา',
  colStatus: 'สถานะ',
  colActions: 'การจัดการ',

  statusActive: 'ใช้งาน',
  statusInvited: 'รอยืนยันตัวตน',
  statusPending: 'รอดำเนินการ',
  statusDisabled: 'ปิดใช้งาน',

  activate: 'เปิดใช้งาน',
  grant: 'มอบหมายสิทธิ์',
  revoke: 'ยกเลิกสิทธิ์',
  disable: 'ปิดบัญชี',
  close: 'ปิด',

  // Create dialog
  createTitle: 'สร้างบัญชีผู้ใช้ใหม่',
  name: 'ชื่อ-นามสกุล',
  username: 'รหัสผู้ใช้ (ไม่บังคับ)',
  phone: 'เบอร์โทรศัพท์ (ใช้เข้าสู่ระบบ)',
  email: 'อีเมล (ไม่บังคับ)',
  password: 'รหัสผ่าน',
  role: 'บทบาท',
  branch: 'สาขา',
  orgWide: 'ทั่วทั้งองค์กร (ทุกสาขา)',
  createSubmit: 'สร้างผู้ใช้',
  creating: 'กำลังสร้าง...',

  // Activate dialog
  activateTitle: 'เปิดใช้งานบัญชี',
  activateIntro: 'ผู้ใช้นี้ยังไม่ได้ผูกกับระบบยืนยันตัวตน กรุณากรอกรหัสผ่านเพื่อเปิดใช้งาน',
  activateSubmit: 'ยืนยันเปิดใช้งาน',
  activating: 'กำลังเปิดใช้งาน...',

  // Grant dialog
  grantTitle: 'มอบหมายบทบาทใหม่',
  grantSubmit: 'มอบหมายสิทธิ์',
  granting: 'กำลังบันทึก...',

  successCreated: 'สร้างบัญชีผู้ใช้เรียบร้อยแล้ว',
  successActivated: 'เปิดใช้งานบัญชีเรียบร้อยแล้ว',
  successGranted: 'มอบหมายสิทธิ์เรียบร้อยแล้ว',
  successRevoked: 'ยกเลิกสิทธิ์เรียบร้อยแล้ว',
  successDisabled: 'ปิดการใช้งานบัญชีเรียบร้อยแล้ว',
};

export const en = {
  label: 'Users & Permissions',
  eyebrow: 'ORGANIZATION & USERS',
  title: 'Users & Permissions',
  subtitle: 'Manage user accounts, roles, scopes, and access',

  heading: 'User Accounts',
  intro: 'ADMINs can create users, grant or revoke roles, and disable accounts.',
  createButton: 'Create User',
  refresh: 'Refresh',
  noUsers: 'No users found',
  loading: 'Loading...',

  colName: 'Name / ID',
  colRoles: 'Roles & Scopes',
  colStatus: 'Status',
  colActions: 'Actions',

  statusActive: 'Active',
  statusInvited: 'Invited',
  statusPending: 'Pending',
  statusDisabled: 'Disabled',

  activate: 'Activate',
  grant: 'Grant Role',
  revoke: 'Revoke Role',
  disable: 'Disable',
  close: 'Close',

  // Create dialog
  createTitle: 'Create New User',
  name: 'Full Name',
  username: 'Username (optional)',
  phone: 'Phone Number (for sign-in)',
  email: 'Email (optional)',
  password: 'Password',
  role: 'Role',
  branch: 'Branch',
  orgWide: 'Organization-wide (All branches)',
  createSubmit: 'Create User',
  creating: 'Creating...',

  // Activate dialog
  activateTitle: 'Activate User Account',
  activateIntro: 'This user is not yet bound to Identity. Enter a password to activate.',
  activateSubmit: 'Activate',
  activating: 'Activating...',

  // Grant dialog
  grantTitle: 'Grant Role',
  grantSubmit: 'Grant',
  granting: 'Saving...',

  successCreated: 'User created successfully',
  successActivated: 'User activated successfully',
  successGranted: 'Role granted successfully',
  successRevoked: 'Role revoked successfully',
  successDisabled: 'User disabled successfully',
};
