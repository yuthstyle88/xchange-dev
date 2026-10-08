// V1-devices-screen (#404): the screen's own text table (design W6, W10). Written before the
// implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests
// in a file of your own.
//
// The keys below are the contract the screen tests read through (`th.<key>`). Each value of
// V2_THAI is v2's text verbatim (`xchange-v2-source@4d0bdbb:src/devices-public/devices.html`
// lines 2-10 and `devices.js` line 8, ticket §10.1 D3-D13, D16). TICKET_THAI is the wording the
// ticket gives for what v3 says instead of v2 (§10.3 V6, V9, and §7 `โหลดเพิ่ม`); a v2 sentence
// with a value in it is one key with a named placeholder.
//
// The status and kind labels (D14, D15, V7) are not keyed here: their Thai is pinned through
// the rendered screen in `DevicesScreen.test.tsx`, so their keys are the implementer's.
import { describe, expect, it } from 'vitest';
import { collectScreens } from '../../shared/i18n';
import * as table from './i18n';
import { en, th } from './i18n';

/** Key → v2's text, verbatim. */
const V2_THAI: Record<string, string> = {
  // D3, D4
  adminLink: 'แอดมินหน้าร้าน ↗',
  deskLink: 'ทำรายการ ↗',
  eyebrow: 'FRONT DESK → BACK OFFICE',
  heading: 'งานหน้าร้านและอุปกรณ์',
  intro: 'ทุกงานผูกกับสาขา เลขรายการ ผู้ทำงาน และเวลา',
  branch: 'สาขา',
  // D6
  metricThb: 'THB · ยอดระบบ',
  metricUsd: 'USD · ยอดระบบ',
  metricCompleted: 'รายการสำเร็จในสาขา',
  // D7
  printerTag: 'SIMULATOR',
  printerName: 'NEOCAL NTP-8220BT',
  printerText: 'สร้างงานพิมพ์สลิปจากรายการที่บันทึกสำเร็จ แสดงตัวอย่างกระดาษ 80 มม. โดยไม่ส่งออก USB',
  printerOnline: 'จำลองเครื่องพิมพ์ออนไลน์',
  printerReady: 'ตัวจำลองพร้อมรับงาน',
  printerOffline: 'ตัวจำลองออฟไลน์ — งานจะล้มเหลวโดยไม่บันทึกว่าพิมพ์สำเร็จ',
  // D8
  scanTag: 'SCAN / IMPORT',
  scanName: 'Brother DCP-T430W',
  scanText: 'สแกนแล้วแนบ JPG / PNG / PDF ผ่านหน้าทำรายการ เอกสารผูกกับลูกค้าและเลขรายการเมื่อบันทึก',
  // D9 (v2's note is two lines joined by <br>)
  counterTag: 'ยืนยันโดยพนักงาน',
  counterName: 'เครื่องนับธนบัตร',
  counterText:
    'หน้าทำรายการต้องยืนยันการนับก่อนบันทึกเงินสด รุ่นอุปกรณ์ยังไม่ทราบ จึงไม่มีการอ้างว่าอ่านจำนวนจากเครื่องได้',
  counterNoteCount: 'จำนวนธนบัตร ≠ มูลค่าเงิน',
  counterNoteCheck: 'ต้องตรวจชนิดราคา สกุล และจำนวนร่วมกัน',
  // D10: the parts V6 keeps
  drawerOpen: 'พนักงานยืนยันว่าเปิด',
  drawerClose: 'พนักงานยืนยันว่าปิด',
  drawerNote: 'ปุ่มเหล่านี้บันทึกการสังเกต ไม่ใช่ผลเซนเซอร์',
  // D11
  requestEyebrow: 'รายการ → งานอุปกรณ์',
  requestHeading: 'สั่งงานจากรายการที่สำเร็จ',
  refresh: 'อัปเดตข้อมูล',
  transaction: 'รายการแลกเงิน',
  choose: 'เลือกรายการ',
  noTransactions: 'ยังไม่มีรายการ',
  reason: 'เหตุผลเปิดลิ้นชัก',
  reasonPlaceholder: 'เช่น ส่งมอบเงินให้ลูกค้าตามรายการ',
  print: 'เพิ่มงานพิมพ์',
  drawer: 'เพิ่มคำสั่งเปิดลิ้นชัก',
  // D12
  colJobTime: 'เวลา / เลขงาน',
  colJobKind: 'ประเภท / รายการ',
  colJobActor: 'ผู้สั่ง',
  colJobStatus: 'สถานะ',
  colJobActions: 'ดำเนินการ',
  noRef: 'ไม่ผูกกับรายการ',
  process: 'จำลองดำเนินการ',
  retry: 'ลองใหม่',
  receipt: 'ตัวอย่างสลิป 80 มม.',
  noJobs: 'ยังไม่มีงานอุปกรณ์ บันทึกรายการจากหน้าร้านแล้วงานพิมพ์จะขึ้นที่นี่',
  // D13
  eventsEyebrow: 'BACK OFFICE AUDIT',
  eventsHeading: 'บันทึกเหตุการณ์หลังบ้าน',
  exportCsv: 'ดาวน์โหลดบันทึก CSV',
  colTime: 'เวลา',
  colActor: 'ผู้ทำงาน',
  colEvent: 'เหตุการณ์',
  colRef: 'เลขอ้างอิง',
  colDetail: 'รายละเอียด',
  noEvents: 'ยังไม่มีเหตุการณ์',
  // D16
  footer: 'ก่อนต่ออุปกรณ์จริง ต้องมี device agent บนเครื่องหน้าร้าน ระบบสิทธิ์ผู้ใช้ และทดสอบไดรเวอร์ตามเอกสารส่งมอบ',
};

/** Key → the ticket's wording for what v3 says instead (§10.3 V6, V9; §7 `โหลดเพิ่ม`). */
const TICKET_THAI: Record<string, string> = {
  bannerSimulated: 'เครื่องพิมพ์ของสาขานี้เป็นตัวจำลอง · ไม่ส่งคำสั่งไปเครื่องพิมพ์หรือลิ้นชักจริง',
  bannerNetwork: 'งานพิมพ์สลิปยังเป็นการจำลอง · คำสั่งเปิดลิ้นชักส่งไปที่เครื่องพิมพ์ของสาขา',
  drawerAgent: 'คำสั่งเปิดลิ้นชักส่งให้ device agent ของสาขา',
  drawerNetwork: 'คำสั่งเปิดลิ้นชักส่งไปที่เครื่องพิมพ์ {printerModel}',
  drawerNone: 'ไม่ได้ต่อลิ้นชักกับระบบ · คำสั่งเปิดเป็นการจำลอง',
  eventsNote: 'ข้อมูลล่าสุด 100 เหตุการณ์ต่อหน้า · CSV ส่งออกเฉพาะที่แสดง',
  loadMore: 'โหลดเพิ่ม',
};

/** v2 texts that V6 replaces: no key of the table may carry them. */
const RETIRED = [
  'งานอุปกรณ์ยังเป็นการจำลอง · ไม่ส่งคำสั่งไปเครื่องพิมพ์หรือลิ้นชักจริง',
  'เซนเซอร์ยังไม่ผ่านการทดสอบ',
  'ลิ้นชัก one',
  'ผลจริงที่ทดสอบ: ตอนเปิดและปิดตอบค่าเดียวกัน จึงแยกคำสั่งเปิดกับการเห็นว่าเปิดจริงอย่างชัดเจน',
  'ข้อมูลล่าสุด 200 เหตุการณ์ · CSV ส่งออกเฉพาะที่แสดง',
];

function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort();
}

describe('devices text table (#404, W6, W10)', () => {
  it("th carries v2's text verbatim under each contract key (D3-D13, D16)", () => {
    const thTable = th as Record<string, string>;
    for (const [key, text] of Object.entries(V2_THAI)) {
      expect(thTable[key], key).toBe(text);
    }
  });

  it("th carries the ticket's wording for the texts v3 replaces (V6, V9, โหลดเพิ่ม)", () => {
    const thTable = th as Record<string, string>;
    for (const [key, text] of Object.entries(TICKET_THAI)) {
      expect(thTable[key], key).toBe(text);
    }
  });

  it("no th text is one of v2's texts that V6 and V9 retire", () => {
    const values = Object.values(th as Record<string, string>);
    for (const text of RETIRED) {
      expect(values, text).not.toContain(text);
    }
  });

  it('en has exactly the keys of th, each with the same placeholders and a text without Thai', () => {
    const thTable = th as Record<string, string>;
    const enTable = en as Record<string, string>;
    expect(Object.keys(enTable).sort()).toEqual(Object.keys(thTable).sort());
    for (const key of Object.keys(thTable)) {
      expect(placeholders(enTable[key]!), key).toEqual(placeholders(thTable[key]!));
      expect(enTable[key]!.length, key).toBeGreaterThan(0);
      expect(/[฀-๿]/.test(enTable[key]!), `${key}: en has no Thai`).toBe(false);
    }
  });

  it('the shared collector takes the table under the namespace devices', () => {
    const tables = collectScreens({ '/src/screens/devices/i18n.ts': table });
    expect(tables.th['devices.heading']).toBe(V2_THAI.heading);
    expect(tables.th['devices.drawerNetwork']).toBe(TICKET_THAI.drawerNetwork);
    expect(typeof tables.en['devices.heading']).toBe('string');
  });
});
