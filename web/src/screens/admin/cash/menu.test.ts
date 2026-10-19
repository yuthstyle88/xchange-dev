// V2-cash (#467): the cash menu's record, its registration through the shell's glob, its own text
// table, and the W7 / ApiClient rules on its source (ticket §4 criteria 2–4, §7; design §12.3,
// W6, W7, W10). Written before the implementation by the lane controller (claude/7ca80c). Do not
// edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`): the served nav row `เงินสดและกะ` `▣` in group เงินและรายงาน
// (`src/devices-public/admin-workspace.js:11-20`), the title / subtitle of `:90`, the eyebrow of
// v2's `pages.cash` (decoded `admin-html:112`), not dated (`admin-workspace.js:146`); the texts of
// the cash section (`admin-html:20`, `:101-108`), its dialogs (`:125-130`, `:145`), the opening
// picker (`admin-opening-currencies.js:6-24`) and the stock filter and history
// (`admin-workspace.js:122-135`), Thai verbatim (W6).
//
// Panel A5 — the valuation and profit calculators of the shift detail — is not this ticket's
// (controller, issuecomment-6066362902): the table holds none of their texts.
//
// The menu's files are read through eager globs, so on a branch without them each test fails on
// an assertion that the file exists, not on the import.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the menu record's `roles: 'all'` pin → `needs: 'all'` (#1030 §3.10: `MenuDef.roles` becomes `needs`). Nothing else changes.
import { describe, expect, it } from 'vitest';
import type { JSX } from 'solid-js';
import { screenLookup, setLang } from '../../../shared/i18n';
import { menus, type AdminMenu, type AdminMenuProps } from '../menus';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
interface TextsModule {
  th: Record<string, string>;
  en: Record<string, string>;
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });
const textFiles = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });
const sources = import.meta.glob<string>('./**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function menuModule(): MenuModule {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/cash/menu.ts exists').toBeDefined();
  return m!;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/cash/i18n.ts exists').toBeDefined();
  return m!;
}

/** The cash menu's own texts, v2's Thai verbatim (W6), each a value of the menu's table. */
const CASH_TEXTS = [
  // the menu record: label, heading (admin-workspace.js:18, :90; admin-html:112)
  'เงินสดและกะ',
  'CASH & SHIFTS',
  'เปิดกะ รับ–จ่าย ตรวจนับ และติดตามประวัติในสาขาเดียวกัน',
  // the status banner, admin-html:101
  'ยังไม่มีกะเปิด',
  'ยังไม่มีกะเปิด — พนักงานนับเงินที่หน้าพนักงาน › กะของฉัน › นับเปิดกะ แล้วผู้มีสิทธิ์รับรองยืนยันยอด กะจึงเปิด',
  // the four stat cards, admin-html:102
  'เงินสดตั้งต้น',
  'ยอด THB ที่ตรวจนับตอนเปิดกะ',
  'เงินสดรับเข้า',
  'จากแลกเงินและรับเงินเข้าลิ้นชัก',
  'เงินสดจ่ายออก',
  'จ่ายให้ลูกค้าและจ่ายนอกการแลกเงิน',
  'เงินสด THB คงเหลือ',
  'ตั้งต้น + รับเข้า − จ่ายออก',
  // the reference summary, admin-html:103, :144
  'ส่วนต่างเทียบเรทอ้างอิงในกะ',
  'ยังไม่มีเรทอ้างอิง',
  // the stock card and its buttons, admin-html:20, :105
  'CASH & CURRENCY STOCK',
  'เงินสดและสต็อกตามกะ',
  'ตั้งเงินสดเริ่มต้น / เปิดกะ',
  'ดูตัวอย่าง 500,000',
  'รับ–จ่ายเงินสด',
  'ตรวจนับและปิดกะ',
  'สกุลเงิน',
  'ยกมาตอนเปิดกะ',
  'เคลื่อนไหวสุทธิ',
  'คงเหลือตามระบบ',
  'เริ่มจากยอดเงินที่ตรวจนับจริง',
  'รายการก่อนเปิดกะจะไม่ถูกนำมาหักซ้ำจากยอดยกมา',
  // the stock filter and the history, admin-workspace.js:123, :134
  'แสดงทุกสกุล รวมยอดศูนย์',
  'ปกติแสดง THB สกุลที่ตั้งเรท และสกุลที่มียอดหรือการเคลื่อนไหว',
  'ประวัติรับ–จ่ายและกะที่ผ่านมา',
  // movements, admin-html:20, :106
  'ประวัติรับ–จ่ายนอกการแลกเงิน',
  'วัน / เวลา',
  'กะ',
  'รายการ',
  'จำนวน',
  'เหตุผล',
  'รับเข้า',
  'จ่ายออก',
  'ยังไม่มีรายการรับ–จ่าย',
  // shifts, admin-html:20, :107
  'ประวัติกะ',
  'เปิดเมื่อ',
  'ผู้เปิด',
  'สถานะ',
  'ผลต่างตอนปิด',
  'กำลังเปิด',
  'ปิดแล้ว',
  'ตรงตามระบบ',
  'ยังไม่มีประวัติกะ',
  // the footnote, admin-html:20
  'เงินสด THB ขยับเฉพาะรายการชำระเงินสด เงินต่างประเทศขยับทั้งรายการเงินสดและโอนธนาคาร ค่าธรรมเนียมธนาคารไม่หักจากลิ้นชักโดยอัตโนมัติ',
  // the open dialog, admin-html:127 and admin-opening-currencies.js:6, :10, :19, :24
  'ค้นหาและเพิ่มสกุลเงิน',
  'เช่น USD, ดอลลาร์, EUR',
  'ไม่พบสกุลเงิน หรือเพิ่มสกุลนี้แล้ว',
  'ลบ',
  'ยกเลิก',
  'ยืนยันเปิดกะ',
  // the movement dialog, admin-html:128
  'บันทึกรับ–จ่ายเงินสด',
  'สำหรับนำเงินเข้าลิ้นชัก นำฝาก หรือค่าใช้จ่ายเงินสด ไม่ต้องบันทึกซ้ำรายการแลกเงิน ระบบนี้บันทึกบัญชีเท่านั้น ไม่ได้โอนเงินจริง',
  'ประเภท',
  'เหตุผล / อ้างอิง',
  'เช่น เงินเติมลิ้นชัก หรือ นำเงินฝากธนาคาร',
  'บันทึกรับ–จ่าย',
  'กรุณาระบุเหตุผล',
  // the close dialog, admin-html:125, :129
  'THB · นับได้จริง',
  'ตามระบบ',
  'เหตุผลยอดต่าง / หมายเหตุ',
  'ต้องระบุเมื่อมีเงินขาดหรือเกิน',
  'กลับไปตรวจนับ',
  'ยืนยันปิดกะ',
  // the shift detail, admin-html:130
  'ผลตรวจนับกะที่ปิดแล้ว',
  'รายละเอียดกะที่เปิด',
  'นับจริง',
  'ผลต่าง',
  'ปิด',
  // the example, admin-html:145
  'ตัวอย่างเงินสดตั้งต้น 500,000 บาท',
  'ตัวอย่างคำนวณเท่านั้น ไม่บันทึกหรือเปลี่ยนยอดของคุณ',
  'เข้าใจแล้ว',
] as const;

/** Panel A5's texts (`shift-valuation.js:9`, `:13-14`; `profit-summary.js:7-13`): not this ticket's. */
const A5_TEXTS = [
  'กำไรส่วนต่างจากการตีราคา (ประมาณการ)',
  'เปิดอัตราแลกเปลี่ยน ธปท. ↗',
  'วันที่เรทที่ ธปท. เผยแพร่',
  'ประเภทเรท',
  'กรอกเรทอ้างอิง ธปท.',
  'คำนวณส่วนต่าง',
  'เงินคงเหลือรวมเป็นบาท',
  'ส่วนต่างหลังตัดผลรับ–จ่ายอื่น',
  'กำไรและยอดถอนได้',
  'กำไรที่เกิดขึ้นแล้ว',
  'ส่วนต่างตีราคา',
  'ยอดถอนได้',
  'รอข้อมูล',
  'ข้อมูลที่ใช้คำนวณ',
  'คำนวณกำไรและยอดถอนได้',
  'ข้อมูลเปลี่ยน กรุณาคำนวณใหม่',
] as const;

describe('V2-cash: the menu record (#467 §7; design §12.3)', () => {
  it("is the served nav's cash row: money, order 4, เงินสดและกะ ▣, not dated, roles 'all', v2's heading", () => {
    const { menu } = menuModule();
    setLang('th');
    expect(menu.id).toBe('cash');
    expect(menu.group).toBe('money');
    expect(menu.order).toBe(4);
    expect(menu.icon).toBe('▣');
    expect(menu.label()).toBe('เงินสดและกะ');
    expect(menu.dated).toBe(false);
    expect(menu.needs).toBe('all');
    expect(menu.heading.eyebrow()).toBe('CASH & SHIFTS');
    expect(menu.heading.title()).toBe('เงินสดและกะ');
    expect(menu.heading.subtitle()).toBe('เปิดกะ รับ–จ่าย ตรวจนับ และติดตามประวัติในสาขาเดียวกัน');
    expect(typeof menu.Component).toBe('function');
  });

  it("reaches the shell through menus.ts's glob, with no shell file edited", () => {
    const { menu } = menuModule();
    const listed = menus.filter((m) => m.id === 'cash');
    expect(listed.length, 'cash is listed once by ./*/menu.ts').toBe(1);
    expect(listed[0]).toBe(menu);
    const before = menus.filter((m) => m.order < 4).map((m) => m.id);
    const after = menus.filter((m) => m.order > 4).map((m) => m.id);
    expect(menus.map((m) => m.id)).toEqual([...before, 'cash', ...after]);
  });

  it('follows setLang: label and heading switch to the en table', () => {
    const { menu } = menuModule();
    const { th, en } = texts();
    const keyOf = (text: string): string => {
      const found = Object.entries(th).find(([, v]) => v === text);
      expect(found, `${text} is a value of the menu's th table`).toBeDefined();
      return found![0];
    };
    try {
      setLang('en');
      expect(menu.label()).toBe(en[keyOf('เงินสดและกะ')]);
      expect(menu.heading.eyebrow()).toBe(en[keyOf('CASH & SHIFTS')]);
      expect(menu.heading.subtitle()).toBe(en[keyOf('เปิดกะ รับ–จ่าย ตรวจนับ และติดตามประวัติในสาขาเดียวกัน')]);
    } finally {
      setLang('th');
    }
  });
});

describe("V2-cash: the menu's own text table (W6, W10)", () => {
  it('holds every cash text, Thai verbatim, with an English twin for every key', () => {
    const { th, en } = texts();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    const values = Object.values(th);
    for (const text of CASH_TEXTS) {
      expect(values, `the menu's th table holds ${JSON.stringify(text)}`).toContain(text);
    }
    for (const [key, value] of Object.entries(en)) {
      expect(value.trim(), `en.${key} is not empty`).not.toBe('');
    }
  });

  it('is collected under the namespace admin.cash.*', () => {
    const { th, en } = texts();
    for (const key of Object.keys(th)) {
      expect(screenLookup.th[`admin.cash.${key}`], `admin.cash.${key} (th)`).toBe(th[key]);
      expect(screenLookup.en[`admin.cash.${key}`], `admin.cash.${key} (en)`).toBe(en[key]);
    }
  });

  it('holds none of panel A5’s texts — the calculators are another ticket’s (issuecomment-6066362902)', () => {
    const { th, en } = texts();
    const all = [...Object.values(th), ...Object.values(en)].join('\n');
    for (const text of A5_TEXTS) {
      expect(all.includes(text), `the table holds no ${JSON.stringify(text)}`).toBe(false);
    }
    expect(/\bTODO\b|coming soon|เร็ว ๆ นี้|เร็วๆ นี้|ยังไม่รองรับ/i.test(all), 'no placeholder text').toBe(false);
  });
});

describe('V2-cash: the source does no money arithmetic and calls no fetch (criteria 3, 4; W7)', () => {
  const own = Object.entries(sources).filter(([path]) => !path.includes('.test.'));

  it('has its source files in this folder', () => {
    expect(own.map(([p]) => p)).toContain('./menu.ts');
    expect(own.map(([p]) => p)).toContain('./i18n.ts');
  });

  it('never parses, rounds or scales a figure, and never calls fetch', () => {
    const forbidden = /Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100|fetch\(/;
    for (const [path, text] of own) {
      const lines = text.split('\n');
      lines.forEach((line, i) => {
        expect(forbidden.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
      });
    }
  });

  it('calls no compute action and carries no calculator outside calculators/ (panel A5 is #511\'s)', () => {
    for (const [path, text] of own.filter(([p]) => !p.startsWith('./calculators/'))) {
      expect(text.includes('/api/v3/compute'), `${path}: no compute call`).toBe(false);
      expect(/'compute'|"compute"/.test(text), `${path}: no act('compute', …)`).toBe(false);
      expect(/\bTODO\b|FIXME/.test(text), `${path}: no TODO left for A5`).toBe(false);
    }
  });
});
