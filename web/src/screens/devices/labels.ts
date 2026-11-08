import { screenT } from '../../shared/i18n';

export interface LabelItem {
  th: string;
  en: string;
}

export const STATUS_LABELS: Record<string, LabelItem> = {
  QUEUED: { th: 'รอดำเนินการ', en: 'Queued' },
  SIMULATED_PRINTED: { th: 'จำลองพิมพ์สำเร็จ', en: 'Simulated Printed' },
  COMMAND_SIMULATED: { th: 'จำลองส่งคำสั่ง · ไม่ยืนยันเปิดจริง', en: 'Command Simulated (Not Confirmed)' },
  FAILED: { th: 'จำลองการเชื่อมต่อล้มเหลว', en: 'Simulation Connection Failed' },
  LEASED: { th: 'กำลังส่ง', en: 'Sending' },
  EXPIRED: { th: 'หมดเวลา', en: 'Expired' },
  DRAWER_OPENED: { th: 'เปิดลิ้นชักแล้ว', en: 'Drawer Opened' },
};

export const KIND_LABELS: Record<string, LabelItem> = {
  SCAN_IMPORTED: { th: 'นำเข้าเอกสารสแกน', en: 'Import Scanned Document' },
  COUNT_CONFIRMED: { th: 'พนักงานยืนยันนับเงิน', en: 'Staff Confirmed Cash Count' },
  EXCHANGE_LINKED: { th: 'บันทึกรายการและเชื่อมเอกสาร', en: 'Transaction Recorded & Document Linked' },
  PRINT_RECEIPT: { th: 'คำขอพิมพ์สลิป', en: 'Print Receipt Request' },
  DRAWER_REQUEST: { th: 'คำขอเปิดลิ้นชัก', en: 'Open Drawer Request' },
  DRAWER_MANUAL_OBSERVATION: { th: 'พนักงานรายงานสถานะลิ้นชัก', en: 'Staff Drawer Status Report' },
  SIMULATED_PRINTED: { th: 'จำลองพิมพ์เสร็จ', en: 'Simulated Print Complete' },
  COMMAND_SIMULATED: { th: 'จำลองส่งคำสั่งเปิด', en: 'Simulated Open Command' },
  JOB_RETRY: { th: 'ลองงานที่ล้มเหลวอีกครั้ง', en: 'Retry Failed Job' },
  FAILED: { th: 'งานจำลองล้มเหลว', en: 'Simulated Job Failed' },
  SIMULATED_PRINTER_CONNECTION: { th: 'เปลี่ยนสถานะเครื่องพิมพ์จำลอง', en: 'Change Simulated Printer Status' },
  RECEIPT_IMAGE_ARCHIVED: { th: 'เก็บภาพรายละเอียดใบเสร็จ', en: 'Archive Receipt Details Image' },
  KIOSK_REQUEST: { th: 'ลูกค้าส่งคำขอจาก kiosk', en: 'Customer Kiosk Request' },
  KIOSK_ACCEPTED: { th: 'พนักงานรับคิว kiosk', en: 'Staff Accepted Kiosk Queue' },
  KIOSK_ID_CHECKED: { th: 'ตรวจพาสปอร์ตคิว kiosk', en: 'Check Kiosk Passport' },
  KIOSK_COMPLETED: { th: 'บันทึกรายการจากคิว kiosk', en: 'Record Transaction From Kiosk' },
  KIOSK_CURRENCY_CONFIGURED: { th: 'ตั้งค่าสกุลเงิน kiosk', en: 'Configure Kiosk Currency' },
  LEASED: { th: 'กำลังส่ง', en: 'Sending' },
  EXPIRED: { th: 'หมดเวลา', en: 'Expired' },
  DRAWER_OPENED: { th: 'เปิดลิ้นชักแล้ว', en: 'Drawer Opened' },
  JOB_LEASED: { th: 'device agent รับงาน', en: 'Job Leased by Device Agent' },
  AGENT_BRANCH_MISMATCH: { th: 'device agent ผิดสาขา', en: 'Device Agent Branch Mismatch' },
  DRAWER_SKIPPED: { th: 'ข้ามการเปิดลิ้นชัก (สาขาไม่มีลิ้นชัก)', en: 'Drawer Skipped (No Drawer)' },
  KIOSK_CANCELLED: { th: 'ยกเลิกคิว kiosk', en: 'Kiosk Queue Cancelled' },
  KIOSK_CUSTOMER_DIFFERS: { th: 'ลูกค้าที่บันทึกไม่ตรงกับคิว kiosk', en: 'Saved Customer Differs From Kiosk Request' },
};

export const DETAIL_KEY_LABELS: Record<string, LabelItem> = {
  job: { th: 'งาน', en: 'Job' },
  jobId: { th: 'งาน', en: 'Job' },
  reason: { th: 'เหตุผล', en: 'Reason' },
  mode: { th: 'โหมด', en: 'Mode' },
  physicalAction: { th: 'สั่งอุปกรณ์จริง', en: 'Physical action' },
  physicalOpenConfirmed: { th: 'ยืนยันเปิดจริง', en: 'Physical open confirmed' },
  sensorConfirmed: { th: 'เซนเซอร์ยืนยัน', en: 'Sensor confirmed' },
  state: { th: 'สถานะ', en: 'State' },
  source: { th: 'ที่มา', en: 'Source' },
  online: { th: 'ออนไลน์', en: 'Online' },
  slip: { th: 'สลิป', en: 'Slip' },
  mime: { th: 'ชนิดไฟล์', en: 'File type' },
  submissionId: { th: 'เลขที่ส่ง', en: 'Submission ID' },
  passport: { th: 'พาสปอร์ต', en: 'Passport' },
  currency: { th: 'สกุลเงิน', en: 'Currency' },
  leaseId: { th: 'เลขการรับงาน', en: 'Lease ID' },
  namedBranch: { th: 'สาขาที่ระบุ', en: 'Named branch' },
  queuedBy: { th: 'ผู้สั่งงาน', en: 'Queued by' },
  queuedAt: { th: 'เวลาสั่งงาน', en: 'Queued at' },
  agent: { th: 'device agent', en: 'Device agent' },
  agentId: { th: 'device agent', en: 'Device agent' },
  agentName: { th: 'ชื่อ device agent', en: 'Device agent name' },
  agentVersion: { th: 'รุ่น device agent', en: 'Device agent version' },
  drawerTarget: { th: 'ลิ้นชักที่สั่ง', en: 'Drawer target' },
  leasedAt: { th: 'เวลารับงาน', en: 'Leased at' },
  reportedAt: { th: 'เวลารายงาน', en: 'Reported at' },
  cupsJob: { th: 'งานพิมพ์ CUPS', en: 'CUPS job' },
  foreignAmount: { th: 'ยอดต่างประเทศ', en: 'Foreign amount' },
  thb: { th: 'ยอด THB', en: 'THB amount' },
  rate: { th: 'เรท', en: 'Rate' },
  type: { th: 'ประเภท', en: 'Type' },
  note: { th: 'หมายเหตุ', en: 'Note' },
  paymentMethod: { th: 'วิธีชำระ', en: 'Payment method' },
  documentRef: { th: 'เลขเอกสาร', en: 'Document ref' },
  requestCustomerId: { th: 'ลูกค้าในคำขอ', en: 'Requested customer' },
  savedCustomerId: { th: 'ลูกค้าที่บันทึก', en: 'Saved customer' },
};

export const DETAIL_VALUE_LABELS: Record<string, LabelItem> = {
  true: { th: 'ใช่', en: 'Yes' },
  false: { th: 'ไม่ใช่', en: 'No' },
  OPEN: { th: 'เปิด', en: 'Open' },
  CLOSED: { th: 'ปิด', en: 'Closed' },
  EMPLOYEE_OBSERVATION: { th: 'พนักงานรายงาน', en: 'Staff report' },
  SIMULATION: { th: 'จำลอง', en: 'Simulation' },
};

const labelsTh: Record<string, string> = {};
const labelsEn: Record<string, string> = {};

for (const [k, v] of Object.entries(STATUS_LABELS)) {
  labelsTh[`status.${k}`] = v.th;
  labelsEn[`status.${k}`] = v.en;
}
for (const [k, v] of Object.entries(KIND_LABELS)) {
  labelsTh[`kind.${k}`] = v.th;
  labelsEn[`kind.${k}`] = v.en;
}
for (const [k, v] of Object.entries(DETAIL_KEY_LABELS)) {
  labelsTh[`detailKey.${k}`] = v.th;
  labelsEn[`detailKey.${k}`] = v.en;
}
for (const [k, v] of Object.entries(DETAIL_VALUE_LABELS)) {
  labelsTh[`detailValue.${k}`] = v.th;
  labelsEn[`detailValue.${k}`] = v.en;
}

const t = screenT('devices', { th: labelsTh, en: labelsEn });

export function statusLabel(status: string, _lang?: 'th' | 'en'): string {
  if (!Object.prototype.hasOwnProperty.call(STATUS_LABELS, status)) return status;
  return t(`status.${status}`);
}

export function kindLabel(kind: string, _lang?: 'th' | 'en'): string {
  if (!Object.prototype.hasOwnProperty.call(KIND_LABELS, kind)) return kind;
  return t(`kind.${kind}`);
}

export function detailKeyLabel(key: string, _lang?: 'th' | 'en'): string {
  if (!Object.prototype.hasOwnProperty.call(DETAIL_KEY_LABELS, key)) return key;
  return t(`detailKey.${key}`);
}

export function detailValueLabel(code: string, _lang?: 'th' | 'en'): string {
  if (!Object.prototype.hasOwnProperty.call(DETAIL_VALUE_LABELS, code)) return code;
  return t(`detailValue.${code}`);
}
