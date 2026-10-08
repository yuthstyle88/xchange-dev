// V1-kiosk-currencies (#399): the screen's own text table (design W6, W10). Written before the
// implementation by the lane controller. Do not edit, delete or weaken; add tests in a file of
// your own.
//
// The keys below are the contract the screen tests read through (`th.<key>`). Each Thai value is
// v2's text verbatim (`xchange-v2-source@4d0bdbb:src/devices-public/kiosk-currencies.html` and
// `kbank-reference.js`, ticket §10.1 C2–C14); a v2 sentence that v2 built by joining pieces is
// one key with named placeholders.
import { describe, expect, it } from 'vitest';
import { collectScreens } from '../../shared/i18n';
import * as table from './i18n';
import { en, th } from './i18n';

/** Key → v2's Thai, verbatim. */
const V2_THAI: Record<string, string> = {
  back: '← กลับแอดมินหน้าร้าน',
  heading: 'สกุลเงินและเรทหน้าร้าน',
  intro:
    'สำหรับเจ้าของและผู้ดูแลเรท · เรทที่บันทึกใช้ร่วมกับหน้าทำรายการในสาขานี้ สกุลที่ปิดจะไม่รับคำขอใหม่จากหน้าลูกค้า',
  searchLabel: 'ค้นหาประเทศ ชื่อสกุลเงิน หรือรหัส',
  searchPlaceholder: 'USD / Japan / ญี่ปุ่น / ยูโร',
  found: 'พบ {n} สกุลเงิน',
  enabled: 'เปิดบนหน้าลูกค้า',
  buy: 'ร้านรับซื้อ (THB ต่อ 1 หน่วย)',
  sell: 'ร้านขาย (THB ต่อ 1 หน่วย)',
  save: 'บันทึกเรทและการเปิดใช้งาน',
  savedOn: 'บันทึกแล้ว · เปิดบริการ',
  savedOff: 'บันทึกแล้ว · ปิดบริการ',
  kbCaption: 'KB · เรทธนบัตรกสิกรไทย (ข้อมูลอ้างอิงที่บันทึกไว้)',
  colLabel: 'สกุล / ธนบัตร',
  kbColBuy: 'KB รับซื้อ',
  kbColSell: 'KB ขาย',
  kbColTime: 'เวลาประกาศ',
  kbStamp: '{time} (เวลาไทย) · รอบ {round}',
  kbNote: 'THB ต่อ 1 หน่วย · ยังไม่อัปเดตอัตโนมัติ · — คือไม่มีเรท',
  compareHeading: 'เทียบเรทธนบัตร · กสิกรไทย / หน้าร้าน',
  comparePublished: 'ธนาคารประกาศ: {time} (เวลาไทย) · รอบ {round}',
  compareWarning:
    'ข้อมูลที่ตรวจจากหน้าเว็บธนาคารครั้งล่าสุด — ยังไม่อัปเดตอัตโนมัติ (ข้อมูลวันที่ 2 ต.ค. 2026) โปรดตรวจต้นทางก่อนใช้เทียบเรท',
  compareLink: 'เปิดเรทล่าสุดจากกสิกรไทย ↗',
  compareNote:
    'แสดงเฉพาะสกุลที่กสิกรมีเรทธนบัตรรับซื้อหรือขายในข้อมูลอ้างอิงชุดนี้ · หน่วย THB ต่อ 1 หน่วยเงินต่างประเทศ · ส่วนต่าง = เรทร้าน − เรทธนาคาร · — หมายถึงไม่มีเรท ไม่ใช่ศูนย์ · ร้านยังเป็นผู้กำหนดราคาเอง',
  cmpColBankBuy: 'กสิกรรับซื้อ',
  cmpColShopBuy: 'ร้านรับซื้อ',
  cmpColBuyDiff: 'ต่างรับซื้อ',
  cmpColBankSell: 'กสิกรขาย',
  cmpColShopSell: 'ร้านขาย',
  cmpColSellDiff: 'ต่างขาย',
  kbankFailed: 'โหลดข้อมูลอ้างอิงกสิกรไทยไม่สำเร็จ กรุณาตรวจหน้าเว็บธนาคารโดยตรง',
};

/** Keys whose value this file does not pin (v2's text is English: `aria-label="KBank reference"`). */
const OTHER_KEYS = ['bankReference'];

function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort();
}

describe('kiosk-currencies text table (#399, W6, W10)', () => {
  it("th carries v2's Thai verbatim under each contract key", () => {
    const thTable = th as Record<string, string>;
    for (const [key, text] of Object.entries(V2_THAI)) {
      expect(thTable[key], key).toBe(text);
    }
  });

  it('th has the aria-label key of the KBank section, as text', () => {
    const thTable = th as Record<string, string>;
    for (const key of OTHER_KEYS) {
      expect(typeof thTable[key], key).toBe('string');
      expect(thTable[key]!.length, key).toBeGreaterThan(0);
    }
  });

  it('en has exactly the keys of th, each with the same placeholders and an English text', () => {
    const thTable = th as Record<string, string>;
    const enTable = en as Record<string, string>;
    expect(Object.keys(enTable).sort()).toEqual(Object.keys(thTable).sort());
    for (const key of Object.keys(thTable)) {
      expect(placeholders(enTable[key]!), key).toEqual(placeholders(thTable[key]!));
      expect(enTable[key]!.length, key).toBeGreaterThan(0);
      expect(/[฀-๿]/.test(enTable[key]!), `${key}: en has no Thai`).toBe(false);
    }
  });

  it('the shared collector takes the table under the namespace kiosk-currencies', () => {
    const tables = collectScreens({ '/src/screens/kiosk-currencies/i18n.ts': table });
    expect(tables.th['kiosk-currencies.heading']).toBe(V2_THAI.heading);
    expect(tables.th['kiosk-currencies.found']).toBe(V2_THAI.found);
    expect(typeof tables.en['kiosk-currencies.heading']).toBe('string');
  });
});
