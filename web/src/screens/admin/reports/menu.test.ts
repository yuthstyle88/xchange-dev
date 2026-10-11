// V2-reports (#469): the reports menu's record, its registration through the shell's glob, its
// own text table and the W7 / ApiClient rules on its source (ticket §4 criteria 2–4, §7; design
// §12.3, W6, W7, W10). Written before the implementation by the lane controller (claude/7ca80c).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`; `admin-html:N` = line N of the decoded admin page, ticket §2):
// the served nav row `รายงาน` `▥` in group เงินและรายงาน (`src/devices-public/admin-workspace.js:11-20`),
// the title / subtitle of `:92`, the eyebrow of v2's `pages.reports` (`admin-html:73`), dated
// (`admin-workspace.js:146`); the texts of the reports section (`admin-html:20`, `:69-71`, `:144`),
// of the served tabs and footnote (`admin-workspace.js:100`, `:115-120`) and of the employee
// summary (`employee-summary.js:6`, `:11-17`, `:23`), Thai verbatim (W6).
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
const sources = import.meta.glob<string>('./*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function menuModule(): MenuModule {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/reports/menu.ts exists').toBeDefined();
  return m!;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/reports/i18n.ts exists').toBeDefined();
  return m!;
}

/** The reports' own static texts, v2's Thai verbatim (W6), each a value of the menu's table. */
const TEXTS = [
  // the menu record: label, heading (admin-workspace.js:18, :92; admin-html:73)
  'รายงาน',
  'DAILY REPORT',
  'แยกดูช่องทางชำระเงิน สกุลเงิน และยอดของพนักงาน',
  // the tabs (admin-workspace.js:100, :116-120)
  'ประเภทรายงาน',
  'ช่องทางชำระเงิน',
  'สกุลเงินและส่วนต่าง',
  'แยกพนักงาน',
  // the four stat cards (admin-html:69)
  'รับเงินบาทรวม',
  'ยอดชำระจากรายการร้านขาย',
  'จ่ายเงินบาทรวม',
  'ยอดชำระจากรายการร้านรับซื้อ',
  'ค่าบริการลูกค้า',
  'รวมอยู่ในยอดชำระแล้ว',
  'ค่าธรรมเนียมธนาคาร',
  'แยกจากยอดชำระลูกค้า',
  // the payment report (admin-html:20, :70)
  'SETTLEMENT SUMMARY',
  'สรุปตามช่องทางชำระเงิน',
  'ช่องทาง',
  'จำนวนรายการ',
  'รับเข้า',
  'จ่ายออก',
  'รับเข้า − จ่ายออก',
  // the served footnote (admin-workspace.js:115)
  'รวมเฉพาะรายการสำเร็จตามช่วงวันที่และสาขาที่เลือก · ยอดรับ–จ่ายและส่วนต่างเรทไม่ใช่กำไรสุทธิ',
  // the currency report (admin-html:20, :71)
  'CURRENCY BREAKDOWN',
  'สรุปตามสกุลเงิน',
  'สกุลเงิน',
  'ร้านรับซื้อ (หน่วยเงิน)',
  'ร้านขาย (หน่วยเงิน)',
  'เงินต้นรวม THB',
  'ค่าบริการ THB',
  // the reference comparison (admin-html:103, :144)
  'ยังไม่มีเรทอ้างอิง',
  // the employee summary (employee-summary.js:6, :11-17)
  'สรุปยอดแยกพนักงาน',
  'ดาวน์โหลด CSV',
  'อิงสาขาและช่วงวันที่ด้านบน · รับเข้า/จ่ายออกจากมุมร้าน หน่วย THB · รวมเฉพาะรายการสำเร็จ · ค่าบริการรวมในยอดรับ–จ่ายแล้ว ค่าธรรมเนียมธนาคารแสดงแยก · ยอดสุทธิไม่ใช่กำไร',
  'พนักงาน',
  'ทุกคน',
  'ค้นหาพนักงาน',
  'ชื่อบัญชี / รหัสพนักงาน',
  'ทั้งหมด',
  'สำเร็จ',
  'ค้าง',
  'ยกเลิก',
  'รับเข้า THB',
  'จ่ายออก THB',
  'สุทธิ THB',
  'ค่าบริการ',
  'รับ–จ่ายเงินบาท แยกช่องทาง',
  'เงินต่างประเทศที่รับ–จ่าย',
  'รับเข้า (หน่วยเงิน)',
  'จ่ายออก (หน่วยเงิน)',
  'รายการทั้งหมดของพนักงาน',
  // the employee CSV's header (employee-summary.js:23)
  'เลขรายการ',
  'วันที่',
  'สถานะ',
  'ประเภท',
  'จำนวน',
  'ค่าบริการ THB',
  'ค่าธรรมเนียมธนาคาร THB',
  // the export (admin-workspace.js:147; admin-html:170)
  'ส่งออกรายการ CSV',
  'ไม่มีรายการให้ส่งออก',
] as const;

describe('V2-reports: the menu record (#469 §7; design §12.3)', () => {
  it("is the served nav's reports row: money, order 7, รายงาน ▥, dated, roles 'all', v2's heading", () => {
    const { menu } = menuModule();
    setLang('th');
    expect(menu.id).toBe('reports');
    expect(menu.group).toBe('money');
    expect(menu.order).toBe(7);
    expect(menu.icon).toBe('▥');
    expect(menu.label()).toBe('รายงาน');
    expect(menu.dated).toBe(true);
    expect(menu.needs).toBe('all');
    expect(menu.heading.eyebrow()).toBe('DAILY REPORT');
    expect(menu.heading.title()).toBe('รายงาน');
    expect(menu.heading.subtitle()).toBe('แยกดูช่องทางชำระเงิน สกุลเงิน และยอดของพนักงาน');
    expect(typeof menu.Component).toBe('function');
  });

  it("reaches the shell through menus.ts's glob, with no shell file edited", () => {
    const { menu } = menuModule();
    const listed = menus.filter((m) => m.id === 'reports');
    expect(listed.length, 'reports is listed once by ./*/menu.ts').toBe(1);
    expect(listed[0]).toBe(menu);
    const money = menus.filter((m) => m.group === 'money').map((m) => m.order);
    expect(money).toContain(7);
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
      expect(menu.label()).toBe(en[keyOf('รายงาน')]);
      expect(menu.heading.title()).toBe(en[keyOf('รายงาน')]);
      expect(menu.heading.subtitle()).toBe(en[keyOf('แยกดูช่องทางชำระเงิน สกุลเงิน และยอดของพนักงาน')]);
      expect(menu.label()).not.toBe('รายงาน');
    } finally {
      setLang('th');
    }
  });
});

describe("V2-reports: the menu's own text table (W6, W10)", () => {
  it('holds every reports text, Thai verbatim, with an English twin for every key', () => {
    const { th, en } = texts();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    const values = Object.values(th);
    for (const text of TEXTS) {
      expect(values, `the menu's th table holds ${JSON.stringify(text)}`).toContain(text);
    }
    for (const [key, value] of Object.entries(en)) {
      expect(value.trim(), `en.${key} is not empty`).not.toBe('');
      expect(/[฀-๿]/.test(value), `en.${key} has no Thai: ${value}`).toBe(false);
    }
  });

  it('is collected under the namespace admin.reports.*', () => {
    const { th, en } = texts();
    for (const key of Object.keys(th)) {
      expect(screenLookup.th[`admin.reports.${key}`], `admin.reports.${key} (th)`).toBe(th[key]);
      expect(screenLookup.en[`admin.reports.${key}`], `admin.reports.${key} (en)`).toBe(en[key]);
    }
  });
});

describe('V2-reports: the source does no money arithmetic and calls no fetch (criteria 3, 4; W7)', () => {
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
});
