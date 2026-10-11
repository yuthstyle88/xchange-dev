// V2-rates (#468): the rates menu's record, its registration through the shell's glob, its own
// text table and the W7 / ApiClient rules on its source (ticket §4 criteria 2–4, §7; design
// §12.3, W6, W7, W10). Written before the implementation by the lane controller (claude/7ca80c).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`): the served nav row `เรทและสกุลเงิน` `◎` in group
// เงินและรายงาน (`src/devices-public/admin-workspace.js:11-20`), the title / subtitle of `:91`,
// the eyebrow of v2's `pages.rates` (decoded `admin-html:73`), not dated (`admin-workspace.js:146`);
// the texts of the rates section (`admin-html:19`, its LIVE wording `:100`, `:68`), the LIVE edit
// dialog (`:134`), the BOT panel, the history and the form's BOT lines
// (`src/online/public/rates-reference.js:5-10`) and the link to the kiosk currency page
// (`admin-workspace.js:136-138`), Thai verbatim (W6).
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
  expect(m, 'web/src/screens/admin/rates/menu.ts exists').toBeDefined();
  return m!;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/rates/i18n.ts exists').toBeDefined();
  return m!;
}

/**
 * The rates menu's own fixed texts, v2's Thai verbatim (W6), each in the menu's table.
 *
 * Retired by #602 (RE-5), by its ticket author (claude/e5ab6c), as `docs/design/blueprint/rates.md`
 * §9 lists (the direct save → publications): the texts of what D-3 and §3.3/§4.2 replace are no
 * longer required in the table — the LIVE note "THB ต่อ 1 หน่วย" and the panel's "บาทต่อ 1 หน่วย"
 * (JPY is per 100, §3.1 A1), the server spread header (`published_rates` carries no spread, W7), the
 * LIVE edit dialog and its save button, title and BoT re-fetch notice (`adminSaveRate` is retired),
 * the per-1 JPY note (the reference is shown in BoT's own unit, §4.2), the partner name/cost fields,
 * column and margin (partner quotes become reference data entered after RE-7, §4.1), and the history's
 * BoT-copy and partner columns (A03 reads publications). Every other text stays.
 */
const TEXTS = [
  // the menu record: label, heading (admin-workspace.js:18, :91; admin-html:73)
  'เรทและสกุลเงิน',
  'RATE MANAGEMENT',
  'เรทหน้าร้านและข้อมูลอ้างอิงสำหรับสาขาที่เลือก',
  // the kiosk currency tool, admin-workspace.js:137-138
  'กำหนดสกุลเงินที่ลูกค้าเลือกได้ พร้อมเทียบเรทกสิกร',
  'จัดการสกุลเงินหน้าร้าน →',
  'EXCHANGE RATES',
  'เรทหน้าร้าน',
  'เรทที่ผู้ดูแลกำหนด',
  // the table, admin-html:68
  'สกุลเงิน',
  'ร้านรับซื้อ',
  'ร้านขาย',
  'ยังไม่ตั้ง',
  'แก้ไขเรท',
  'ยกเลิก',
  'เรทร้านขายต้องไม่น้อยกว่าเรทร้านรับซื้อ',
  // the form's BOT line, rates-reference.js:5, :10
  'ยังไม่มีข้อมูลอ้างอิง',
  // the BOT panel, rates-reference.js:7
  'กำลังดึงข้อมูล ธปท.…',
  'โหลดเรท ธปท. ล่าสุด',
  'ประวัติการปรับเรท',
  'แหล่งข้อมูล ธปท. ↗',
  'คู่เงิน',
  'ร้านรับซื้อ / ขาย',
  'ธปท. ซื้อตั๋ว / โอน / ขาย',
  // the history dialog, rates-reference.js:8
  'เวลา / ผู้แก้',
  'คู่เงิน / เรทร้าน',
] as const;

/** The start of each of v2's sentences that carry a value (a code, a time, a figure). */
const TEMPLATES = [
  // admin-html:68
  'แก้ไขล่าสุด ',
  'แก้ไขเรท ',
  // rates-reference.js:5, :7, :8, :10
  'ธปท. วันที่ ',
  'ดึงข้อมูล ',
  'ดึงข้อมูลล่าสุด ',
  'ประวัติการปรับเรท · ',
] as const;

describe('V2-rates: the menu record (#468 §7; design §12.3)', () => {
  it("is the served nav's rates row: money, order 5, เรทและสกุลเงิน ◎, not dated, roles 'all', v2's heading", () => {
    const { menu } = menuModule();
    setLang('th');
    expect(menu.id).toBe('rates');
    expect(menu.group).toBe('money');
    expect(menu.order).toBe(5);
    expect(menu.icon).toBe('◎');
    expect(menu.label()).toBe('เรทและสกุลเงิน');
    expect(menu.dated).toBe(false);
    expect(menu.needs).toBe('all');
    expect(menu.heading.eyebrow()).toBe('RATE MANAGEMENT');
    expect(menu.heading.title()).toBe('เรทและสกุลเงิน');
    expect(menu.heading.subtitle()).toBe('เรทหน้าร้านและข้อมูลอ้างอิงสำหรับสาขาที่เลือก');
    expect(typeof menu.Component).toBe('function');
  });

  it("reaches the shell through menus.ts's glob, with no shell file edited", () => {
    const { menu } = menuModule();
    const listed = menus.filter((m) => m.id === 'rates');
    expect(listed.length, 'rates is listed once by ./*/menu.ts').toBe(1);
    expect(listed[0]).toBe(menu);
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
      expect(menu.label()).toBe(en[keyOf('เรทและสกุลเงิน')]);
      expect(menu.heading.eyebrow()).toBe(en[keyOf('RATE MANAGEMENT')]);
      expect(menu.heading.subtitle()).toBe(en[keyOf('เรทหน้าร้านและข้อมูลอ้างอิงสำหรับสาขาที่เลือก')]);
    } finally {
      setLang('th');
    }
  });
});

describe("V2-rates: the menu's own text table (W6, W10)", () => {
  it('holds every fixed text, Thai verbatim, with an English twin for every key', () => {
    const { th, en } = texts();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    const values = Object.values(th);
    for (const text of TEXTS) {
      const held = values.some((v) => v === text || v.replace(/^[\s·]+/, '') === text.replace(/^[\s·]+/, ''));
      expect(held, `the menu's th table holds ${JSON.stringify(text)}`).toBe(true);
    }
    for (const [key, value] of Object.entries(en)) {
      expect(value.trim(), `en.${key} is not empty`).not.toBe('');
    }
  });

  it("holds the start of each of v2's sentences that carry a value", () => {
    const { th } = texts();
    const values = Object.values(th);
    for (const start of TEMPLATES) {
      const found = values.filter((v) => v.startsWith(start.trim()));
      expect(found.length, `a text starting ${JSON.stringify(start.trim())}`).toBeGreaterThan(0);
    }
  });

  it('is collected under the namespace admin.rates.*', () => {
    const { th, en } = texts();
    for (const key of Object.keys(th)) {
      expect(screenLookup.th[`admin.rates.${key}`], `admin.rates.${key} (th)`).toBe(th[key]);
      expect(screenLookup.en[`admin.rates.${key}`], `admin.rates.${key} (en)`).toBe(en[key]);
    }
  });
});

describe('V2-rates: the source does no money arithmetic and calls no fetch (criteria 3, 4; W7)', () => {
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

  it('builds its own ApiClient and never reaches /api/reference-rates directly (design §12.16)', () => {
    const all = own.map(([, text]) => text).join('\n');
    expect(all, 'the menu builds its own client (controller decision, #462)').toMatch(/new ApiClient\(/);
    expect(all, 'the BOT panel reads the rates_reference view').not.toMatch(/\/api\/reference-rates/);
  });
});
