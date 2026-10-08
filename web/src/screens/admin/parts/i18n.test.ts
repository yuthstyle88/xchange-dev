// V2-parts (#476): the parts' own text table (design §12.2, W6, W10). Written before the
// implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests
// in a file of your own.
//
// `parts/i18n.ts` exports `th` (as const) and `en: Record<keyof typeof th, string>`; the shared
// collector takes it under the namespace `admin.parts` (W10 point 1). The keys are the
// implementer's: this file pins the Thai **values** (v2's text verbatim, W6, read from
// `xchange-v2-source@4d0bdbb`, the decoded admin page `admin-html:N` and
// `src/online/public/slips.js:N`), and that the DEMO wording is not ported (design §12.5).
// Texts with a value in them (`กำลังโหลดใบเสร็จของ <id>…`, `เอกสารที่บันทึกไว้ · <id>`) are pinned
// through the rendered part in `TransactionDetail.test.tsx`, so their keys and placeholders are
// the implementer's too.
import { describe, expect, it } from 'vitest';
import { collectScreens, screenLookup } from '../../../shared/i18n';
import * as table from './i18n';
import { en, th } from './i18n';

/** v2's text, verbatim, that some part shows. */
const V2_THAI: string[] = [
  // DataTable, admin-html:62
  'ไม่พบรายการในช่วงที่เลือก',
  'เปลี่ยนช่วงวันที่หรือตัวกรอง แล้วลองอีกครั้ง',
  // transaction.ts, admin-html:56, :63
  'ร้านรับซื้อ',
  'ร้านขาย',
  'เงินสด',
  'บัตรเครดิต',
  'พร้อมเพย์',
  'โอนธนาคาร',
  'อื่น ๆ',
  'สำเร็จ',
  'ยกเลิก',
  'รอบันทึกเอกสาร',
  // TransactionTable, admin-html:63 with the LIVE header of :100
  'รายการ / เวลา',
  'ลูกค้า',
  'ประเภท',
  'จำนวนเงิน',
  'ยอดชำระ THB',
  'สถานะ',
  // TransactionDetail, admin-html:77 with the LIVE override of :133
  'เลขรายการ',
  'วันที่ / เวลา',
  'เรท',
  'เงินต้น',
  'ค่าบริการ',
  'ร้านจ่ายลูกค้า',
  'ร้านรับจากลูกค้า',
  'ค่าธรรมเนียมธนาคาร',
  'ช่องทาง',
  'พนักงาน',
  'หมายเหตุ',
  'เรทอ้างอิง',
  'มูลค่าอ้างอิง',
  'ส่วนต่างเทียบเรทอ้างอิง',
  'เหตุผลยกเลิก',
  'ข้อมูลจาก เซิร์ฟเวอร์ · รายละเอียดนี้ไม่ใช่ใบเสร็จ',
  'รายการจริง: ตรวจสอบการคืนเงินกับผู้ดูแล',
  'ปิด',
  // the slip section, slips.js:5, :18
  'สลิป / หลักฐานรับ–จ่าย',
  'เปิดสลิป ↗',
  'ดาวน์โหลด',
  'รายการนี้ไม่มีสลิปแนบ',
  // the receipt section, slips.js:30, :35, :38, :46
  'ใบเสร็จ',
  'เปิดดู ใบเสร็จ',
  'เปิดดู ภาพใบเสร็จ',
  'โหลดใบเสร็จไม่ได้ กรุณาเข้าสู่ระบบแล้วลองใหม่',
  'ยังไม่มีใบเสร็จที่บันทึกไว้สำหรับรายการนี้',
  // Dialog, admin-html:22
  'ปิดหน้าต่าง',
  // pending-write.ts, admin-html:120-123, :142
  'เซิร์ฟเวอร์ ยืนยันการบันทึกแล้ว',
  'ยังไม่ได้รับการยืนยัน',
  'ยังไม่แสดงว่าบันทึกสำเร็จ การลองซ้ำใช้รหัสเดิมเพื่อป้องกันบันทึกซ้ำ กรุณาอย่าปิดหรือโหลดหน้านี้ใหม่',
  'ลองคำขอเดิมซ้ำ',
  'ตรวจสถานะล่าสุด',
  'ตรวจพบรายการบันทึกแล้ว',
  'ยังไม่พบคำขอนี้ใน เซิร์ฟเวอร์',
  'หากข้อความผิดพลาดระบุว่ากะหรือยอดเปลี่ยน ให้ยกเลิกคำขอนี้และตรวจข้อมูลล่าสุดก่อนทำใหม่ หากการเชื่อมต่อหมดเวลา ให้ลองคำขอเดิมซ้ำ',
  'ยกเลิกคำขอที่ยังไม่บันทึก',
  'เซิร์ฟเวอร์ ยืนยันว่ารายการบันทึกแล้ว',
  'ยกเลิกคำขอที่ยังไม่บันทึกแล้ว',
  // NewTransactionButton, admin-html:100, :131
  '＋ ทำรายการแลกเงิน',
  'เปิดกะก่อนทำรายการแลกเงิน',
  'ทำรายการแลกเงินจริง',
  'เปิดหน้าทำรายการเดิมเพื่อยืนยันลูกค้า เอกสาร และการรับ–จ่ายเงิน เมื่อ เซิร์ฟเวอร์ ยืนยันการบันทึก สต็อกในกะนี้จะปรับตามรายการ',
  'เปิดหน้าทำรายการ ↗',
];

/** v2's DEMO wording, which the served LIVE page replaces (design §12.5, admin-html:100, :133). */
const DEMO_ONLY = [
  'ข้อมูลทดลอง · ไม่ใช่ใบเสร็จรับเงินจริง',
  'ลูกค้าทดลอง',
  'ยกเลิกรายการทดลอง',
  '＋ เพิ่มรายการทดลอง',
  'ข้อมูลทดลอง',
];

const THAI = /[฀-๿]/;

function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort();
}

describe('the parts text table (#476, W6, W10)', () => {
  it("th carries every v2 text the parts show, verbatim (W6)", () => {
    const values = Object.values(th as Record<string, string>);
    for (const text of V2_THAI) {
      expect(values, text).toContain(text);
    }
  });

  it('no th text is DEMO wording: the parts are LIVE only (design §12.5)', () => {
    for (const text of Object.values(th as Record<string, string>)) {
      for (const demo of DEMO_ONLY) {
        expect(text.includes(demo), `${text} carries ${demo}`).toBe(false);
      }
    }
  });

  it('en has exactly the keys of th, each with the same placeholders and a text without Thai', () => {
    const thTable = th as Record<string, string>;
    const enTable = en as Record<string, string>;
    expect(Object.keys(enTable).sort()).toEqual(Object.keys(thTable).sort());
    for (const key of Object.keys(thTable)) {
      expect(placeholders(enTable[key]!), key).toEqual(placeholders(thTable[key]!));
      expect(enTable[key]!.length, key).toBeGreaterThan(0);
      expect(THAI.test(enTable[key]!), `${key}: en has no Thai`).toBe(false);
    }
  });

  it('the shared collector takes the table under the namespace admin.parts', () => {
    const tables = collectScreens({ '/src/screens/admin/parts/i18n.ts': table });
    const keys = Object.keys(tables.th);
    expect(keys.length).toBe(Object.keys(th).length);
    for (const key of keys) {
      expect(key.startsWith('admin.parts.'), key).toBe(true);
    }
  });

  it("the page's collected table has the parts under admin.parts and no key under parts. (W10, §12.2)", () => {
    const keys = Object.keys(screenLookup.th);
    const ours = keys.filter((k) => k.startsWith('admin.parts.'));
    expect(ours.length).toBe(Object.keys(th).length);
    expect(keys.filter((k) => k.startsWith('parts.'))).toEqual([]);
  });

  it('parts is not a menu: the folder has no menu.ts (design §12.2)', () => {
    const menus = import.meta.glob('./menu.ts');
    expect(Object.keys(menus)).toEqual([]);
  });
});
