// V2-overview (#461): the overview menu's record, its registration through the shell's glob, its
// own text table and the W7 / ApiClient rules on its source (ticket §4 criteria 2–4, §7; design
// §12.3, W6, W7, W10). Written before the implementation by the lane controller (claude/7ca80c).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`): the served nav row `ภาพรวม` `▦` in group งานหน้าร้าน
// (`src/devices-public/admin-workspace.js:11-20`), the title / subtitle of `:88`, the eyebrow of
// v2's `pages.overview` (decoded `admin-html:73`), dated (`admin-workspace.js:146`); the texts of
// the overview section (`admin-html:17`, `:64-67`) and of the status strip
// (`admin-workspace.js:155`), Thai verbatim (W6).
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
  expect(m, 'web/src/screens/admin/overview/menu.ts exists').toBeDefined();
  return m!;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/overview/i18n.ts exists').toBeDefined();
  return m!;
}

/** The overview's own texts, v2's Thai verbatim (W6), each in the menu's table. */
const TEXTS = [
  // the menu record: label, heading (admin-workspace.js:17, :88; admin-html:73)
  'ภาพรวม',
  'DAILY OVERVIEW',
  'ภาพรวมหน้าร้าน',
  'ติดตามสถานะกะและยอดรายการในช่วงวันที่ที่เลือก',
  // the four stat cards, admin-html:64
  'ยอดแลกเงินรวม',
  'มูลค่าเงินต้นของรายการสำเร็จ',
  'รายการสำเร็จ',
  // activity, admin-html:17, :65
  'EXCHANGE ACTIVITY',
  'สัดส่วนยอดแลกเงิน',
  'มูลค่าเงินต้น / THB',
  'รวมเฉพาะรายการสำเร็จ · ไม่รวมค่าบริการ',
  // THB flow, admin-html:17, :66
  'THB FLOW',
  'รับเข้า − จ่ายออก',
  'รับเข้า',
  'จ่ายออก',
  'ยอดชำระรวมค่าบริการ ไม่ใช่กำไร',
  'ไม่ใช่ยอดเงินสดคงเหลือในลิ้นชัก',
  // latest, admin-html:17
  'LATEST TRANSACTIONS',
  'รายการล่าสุด',
  'ดูทั้งหมด →',
  // the status strip, admin-workspace.js:155
  'สถานะสาขาขณะนี้ · ไม่อิงช่วงวันที่',
  'กะเปิดอยู่',
  'ยังไม่เปิดกะ',
  'ยังโหลดสถานะไม่ได้',
  'ตรวจนับเงินก่อนเริ่มทำรายการ',
  'กดรีเฟรชเพื่อตรวจสอบข้อมูลล่าสุด',
  'ดูเงินสดและกะ →',
  'รายการรอบันทึกทั้งหมด',
  'ตรวจสถานะและเอกสารก่อนบันทึกซ้ำ',
  'ตรวจรายการ →',
] as const;

describe('V2-overview: the menu record (#461 §7; design §12.3)', () => {
  it("is the served nav's overview row: desk, order 1, ภาพรวม ▦, dated, roles 'all', v2's heading", () => {
    const { menu } = menuModule();
    setLang('th');
    expect(menu.id).toBe('overview');
    expect(menu.group).toBe('desk');
    expect(menu.order).toBe(1);
    expect(menu.icon).toBe('▦');
    expect(menu.label()).toBe('ภาพรวม');
    expect(menu.dated).toBe(true);
    expect(menu.needs).toBe('all');
    expect(menu.heading.eyebrow()).toBe('DAILY OVERVIEW');
    expect(menu.heading.title()).toBe('ภาพรวมหน้าร้าน');
    expect(menu.heading.subtitle()).toBe('ติดตามสถานะกะและยอดรายการในช่วงวันที่ที่เลือก');
    expect(typeof menu.Component).toBe('function');
  });

  it("reaches the shell through menus.ts's glob, with no shell file edited", () => {
    const { menu } = menuModule();
    const listed = menus.filter((m) => m.id === 'overview');
    expect(listed.length, 'overview is listed once by ./*/menu.ts').toBe(1);
    expect(listed[0]).toBe(menu);
    expect(menus[0]?.id, 'the lowest order: the menu for an empty or unknown hash').toBe('overview');
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
      expect(menu.label()).toBe(en[keyOf('ภาพรวม')]);
      expect(menu.heading.title()).toBe(en[keyOf('ภาพรวมหน้าร้าน')]);
      expect(menu.heading.subtitle()).toBe(en[keyOf('ติดตามสถานะกะและยอดรายการในช่วงวันที่ที่เลือก')]);
    } finally {
      setLang('th');
    }
  });
});

describe("V2-overview: the menu's own text table (W6, W10)", () => {
  it('holds every overview text, Thai verbatim, with an English twin for every key', () => {
    const { th, en } = texts();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    const values = Object.values(th);
    for (const text of TEXTS) {
      expect(values, `the menu's th table holds ${JSON.stringify(text)}`).toContain(text);
    }
    for (const [key, value] of Object.entries(en)) {
      expect(value.trim(), `en.${key} is not empty`).not.toBe('');
    }
  });

  it('is collected under the namespace admin.overview.*', () => {
    const { th, en } = texts();
    for (const key of Object.keys(th)) {
      expect(screenLookup.th[`admin.overview.${key}`], `admin.overview.${key} (th)`).toBe(th[key]);
      expect(screenLookup.en[`admin.overview.${key}`], `admin.overview.${key} (en)`).toBe(en[key]);
    }
  });
});

describe('V2-overview: the source does no money arithmetic and calls no fetch (criteria 3, 4; W7)', () => {
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
