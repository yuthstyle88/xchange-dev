export const th = {
  label: 'พนักงานและสิทธิ์',
  eyebrow: 'TEAM & OPERATIONS',
  title: 'พนักงานและสิทธิ์',
  subtitle: 'บัญชีพนักงานและการตั้งค่าขั้นตอนรับ–ส่งเงิน',

  heading: 'บัญชีพนักงาน',
  intro: 'พนักงานแต่ละคนใช้รหัสผ่านของตนเอง ผู้ส่ง ผู้รับ และผู้ตรวจต้องเป็นคนละคนตามบทบาท',
  add: 'สร้างบัญชีพนักงาน',
  refresh: 'รีเฟรช',
  staffLink: 'เปิดหน้าพนักงาน ↗',

  headerAccountName: 'รหัสบัญชี / ชื่อ',
  headerRole: 'บทบาท',
  headerBranch: 'สาขา',
  headerStatus: 'สถานะ',
  headerActions: '',

  active: 'ใช้งาน',
  mustChange: 'รอเปลี่ยนรหัสผ่าน',
  disabled: 'ปิดใช้งาน',
  reset: 'รีเซ็ตรหัสผ่าน',
  disable: 'ปิดบัญชี',

  createTitle: 'สร้างบัญชีพนักงาน',
  name: 'ชื่อพนักงาน',
  username: 'รหัสบัญชี',
  role: 'บทบาท',
  branch: 'สาขา',
  password: 'รหัสผ่านชั่วคราว',
  handOver: 'ส่งมอบรหัสผ่านให้พนักงานโดยตรง ต้องเปลี่ยนรหัสผ่านก่อนเริ่มงาน',
  createSubmit: 'สร้างบัญชี',

  resetTitle: 'รีเซ็ตรหัสผ่าน',
  newPassword: 'รหัสผ่านชั่วคราวใหม่',
  sessionsEnd: 'เซสชันเดิมทั้งหมดจะถูกยกเลิก',
  resetSubmit: 'ยืนยันรีเซ็ต',

  disableConfirm: 'ปิดบัญชีนี้และยกเลิกเซสชันทั้งหมด?',

  workflow: 'ตั้งค่าขั้นตอนรับ–ส่งเงินของสาขา',
  stateOn: 'สาขานี้เปิดใช้ขั้นตอนรับ–ส่งเงินแล้ว',
  stateOff: 'สาขานี้ยังใช้ขั้นตอนเดิม สร้างบัญชีครบ 6 บทบาทและปิดกะเดิมก่อนเปิดใช้',
  entityButton: 'ยืนยันกิจการและจุดบริการ',
  enableButton: 'เปิดใช้สำหรับสาขานี้',
  enableTitle: 'เปิดใช้ขั้นตอนรับ–ส่งเงิน · ',
  enableNote1:
    'ต้องมีบัญชี Front, Backend, Manager, Rates, Monitor และ Finance ในสาขา และไม่มีกะเดิมเปิดอยู่ หลังเปิดใช้ ปุ่มเปิดกะ รับ–จ่ายเงินสด และปิดกะเดิมจะถูกล็อก ให้พนักงานทำรายการและลงชื่อผ่านหน้าปฏิบัติงาน',
  enableNote2:
    'Finance ต้องกำหนดเงินสดเป้าหมายและแจ้งยอดนับเซฟจริงก่อนเริ่มใช้งาน ไม่มียอดเงินตัวอย่างถูกเติมให้อัตโนมัติ',
  enableSubmit: 'เปิดใช้ขั้นตอนใหม่',
  enabledToast: 'เปิดใช้ขั้นตอนรับ–ส่งเงินแล้ว',
  entityTitle: 'ยืนยันกิจการของสาขา',
  company: 'ชื่อนิติบุคคล',
  site: 'จุดบริการ',
  separated: 'ยืนยันว่ารายการและยอดเงินของสาขานี้เป็นของกิจการนี้ และแยกจากกิจการอื่นแล้ว',
  entityNote: 'การตั้งชื่อนี้ไม่ย้ายหรือแยกธุรกรรมเก่า หากบัญชีเดิมปะปน ให้สร้างสาขาใหม่ก่อน',
  entitySubmit: 'บันทึก',
};

export const en = {
  label: 'Staff & Roles',
  eyebrow: 'TEAM & OPERATIONS',
  title: 'Staff & Roles',
  subtitle: 'Employee accounts and custody workflow settings',

  heading: 'Staff Accounts',
  intro:
    'Each staff member uses their own password. Senders, receivers, and reviewers must be different people according to their roles.',
  add: 'Create Staff Account',
  refresh: 'Refresh',
  staffLink: 'Open Staff Screen ↗',

  headerAccountName: 'Account / Name',
  headerRole: 'Role',
  headerBranch: 'Branch',
  headerStatus: 'Status',
  headerActions: '',

  active: 'Active',
  mustChange: 'Password change required',
  disabled: 'Disabled',
  reset: 'Reset password',
  disable: 'Disable account',

  createTitle: 'Create Staff Account',
  name: 'Staff Name',
  username: 'Account ID',
  role: 'Role',
  branch: 'Branch',
  password: 'Temporary Password',
  handOver:
    'Hand over the password directly to the staff member. Password must be changed before starting work.',
  createSubmit: 'Create Account',

  resetTitle: 'Reset Password',
  newPassword: 'New Temporary Password',
  sessionsEnd: 'All existing sessions will be terminated',
  resetSubmit: 'Confirm Reset',

  disableConfirm: 'Disable this account and terminate all sessions?',

  workflow: 'Branch Cash Handling Workflow Settings',
  stateOn: 'This branch has enabled the cash handling workflow.',
  stateOff:
    'This branch is still using the old procedure. Create accounts for all 6 roles and close old shifts before enabling.',
  entityButton: 'Confirm Entity & Service Site',
  enableButton: 'Enable for this branch',
  enableTitle: 'Enable Cash Handling Workflow · ',
  enableNote1:
    'Must have Front, Backend, Manager, Rates, Monitor, and Finance accounts in this branch, and no open shifts. After enabling, buttons for opening shifts, cash in/out, and old closing shifts will be locked. Staff will operate and sign via the staff screen.',
  enableNote2:
    'Finance must set cash targets and report actual safe counts before starting. No sample funds will be filled automatically.',
  enableSubmit: 'Enable New Workflow',
  enabledToast: 'Cash handling workflow enabled',
  entityTitle: 'Confirm Branch Entity',
  company: 'Company Legal Name',
  site: 'Service Site',
  separated:
    'Confirm that transactions and funds of this branch belong to this entity and are separated from other entities.',
  entityNote:
    'Naming here does not move or separate historical transactions. If past accounts are mixed, please create a new branch first.',
  entitySubmit: 'Save',
};
