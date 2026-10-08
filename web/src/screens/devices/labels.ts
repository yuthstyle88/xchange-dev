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

const t = screenT('devices', { th: labelsTh, en: labelsEn });

export function statusLabel(status: string, _lang?: 'th' | 'en'): string {
  if (!Object.prototype.hasOwnProperty.call(STATUS_LABELS, status)) return status;
  return t(`status.${status}`);
}

export function kindLabel(kind: string, _lang?: 'th' | 'en'): string {
  if (!Object.prototype.hasOwnProperty.call(KIND_LABELS, kind)) return kind;
  return t(`kind.${kind}`);
}
