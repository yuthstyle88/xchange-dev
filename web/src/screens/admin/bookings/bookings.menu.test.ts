// V2-bookings (#463): the menu record and the menu's own text table (ticket §7 `menu.ts`, "Builds
// on" W10; design §12.3 in PR 474). Written before the implementation by the lane controller
// (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// `menu.ts` and `i18n.ts` are read through eager globs, so on the tests-first branch each test
// fails on the assertion `… exists` instead of the file failing to typecheck.
//
// The keys of `i18n.ts` are the implementer's: this file pins the Thai **values** — v2's text
// verbatim (W6), read from `xchange-v2-source@4d0bdbb` `src/online/public/bookings.js:N` and the
// decoded admin page `admin-html:19` — and the words design §12.17 gives the create form. Texts
// with a value in them (the info line, the confirm and cancel sentences, the settings title) are
// pinned through the rendered menu in `bookings.screen.test.tsx`.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the menu record's `roles: 'all'` pin → `needs: 'all'` (#1030 §3.10: `MenuDef.roles` becomes `needs`). Nothing else changes.
import { describe, expect, it } from 'vitest';
import { collectScreens, screenLookup, setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { menus } from '../menus';

const loadedMenu = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
const loadedTable = import.meta.glob<{ th: Record<string, string>; en: Record<string, string> }>(
  './i18n.ts',
  { eager: true },
);

function theMenu(): AdminMenu {
  const m = loadedMenu['./menu.ts'];
  expect(m, 'web/src/screens/admin/bookings/menu.ts exists').toBeDefined();
  expect(m!.menu, 'menu.ts exports `menu`').toBeDefined();
  return m!.menu;
}

function theTable(): { th: Record<string, string>; en: Record<string, string> } {
  const t = loadedTable['./i18n.ts'];
  expect(t, 'web/src/screens/admin/bookings/i18n.ts exists').toBeDefined();
  expect(t!.th, 'i18n.ts exports `th`').toBeDefined();
  expect(t!.en, 'i18n.ts exports `en`').toBeDefined();
  return t!;
}

const THAI = /[฀-๿]/;

/** v2's words the menu shows, verbatim (W6). */
const V2_THAI: string[] = [
  // the served nav and heading (admin-workspace.js:11-20, :96; bookings.js:2's eyebrow)
  'การจอง',
  'ตรวจการจอง ล็อกเรท และส่งต่อไปทำรายการหน้าร้าน',
  // the toolbar and search (admin-html:19)
  'เปิดเว็บจองสำหรับลูกค้า ↗',
  'โหลดการจองล่าสุด',
  'ตั้งค่าการจอง',
  'ค้นหาการจอง',
  'เลขจอง / ชื่อ / เบอร์โทร',
  // loading and empty (bookings.js:6, :10)
  'กำลังโหลดการจอง…',
  'ยังไม่มีการจองที่ตรงกับคำค้นในสาขานี้',
  // the info line's two states (bookings.js:9)
  'เปิดรับจองเมื่อสาขาเปิดกะและตั้งเรทแล้ว',
  'ปิดรับจองใหม่',
  // the table (bookings.js:10)
  'เลขจอง / ลูกค้า',
  'รายการ',
  '~เรท',
  'หมดอายุ',
  'สถานะ',
  'รอพนักงานกำหนด',
  'ยืนยันเรทและเวลา',
  'ทำรายการหน้าร้าน ↗',
  'ยกเลิก',
  // the statuses (bookings.js:4)
  'Member · รอยืนยัน',
  'ล็อกเรทแล้ว',
  'ทำรายการแล้ว',
  // confirm (bookings.js:12)
  'ยืนยันการจอง Member',
  'ยังไม่ตั้งเรท',
  'วันเวลาหมดอายุ (เวลาของเครื่องนี้)',
  'กลับ',
  'ยืนยันล็อกเรท',
  // settings (bookings.js:16)
  'ล็อกเรทลูกค้าใหม่ (นาที)',
  'เปิดรับจองเมื่อมีกะเปิดและเรทพร้อม',
  'Member ให้พนักงานกำหนดเวลาหมดอายุแต่ละรายการ',
  'บันทึก',
  // cancel (bookings.js:18)
  'ยกเลิกการจอง',
  'ยืนยันยกเลิก',
  // the create form's words that v2 already has (design §12.17)
  'ชื่อ',
  'รหัสลูกค้า',
  'เบอร์ติดต่อ',
  'ประเภท',
  'จำนวนเงิน',
  'เรท',
  'สกุลเงิน',
  // the create form's checks, the §5.6.3 sentences (design §12.17)
  'กรอกข้อมูลให้ครบและไม่เกินกำหนด',
  'เบอร์ติดต่อไม่ถูกต้อง',
  'จำนวนเงินหรือเรทไม่ถูกต้อง',
];

describe('V2-bookings: the menu record (#463 §7; design §12.3)', () => {
  it("is the served nav's bookings menu: desk, order 3, การจอง ▤, no period toolbar, roles 'all'", () => {
    setLang('th');
    const menu = theMenu();
    expect(menu.id).toBe('bookings');
    expect(menu.group).toBe('desk');
    expect(menu.order).toBe(3);
    expect(menu.icon).toBe('▤');
    expect(menu.dated).toBe(false);
    expect(menu.needs).toBe('all');
    expect(menu.label()).toBe('การจอง');
    expect(menu.heading.eyebrow()).toBe('ONLINE BOOKINGS');
    expect(menu.heading.title()).toBe('การจอง');
    expect(menu.heading.subtitle()).toBe('ตรวจการจอง ล็อกเรท และส่งต่อไปทำรายการหน้าร้าน');
    expect(typeof menu.Component).toBe('function');
  });

  it("reaches the shell through menus.ts's glob, with no shell file edited", () => {
    const menu = theMenu();
    const listed = menus.find((m) => m.id === 'bookings');
    expect(listed, 'menus.ts lists the bookings folder').toBeDefined();
    expect(listed).toBe(menu);
  });

  it('switches its label and heading with the language (through the menu table)', () => {
    const menu = theMenu();
    const { th, en } = theTable();
    const key = Object.keys(th).find((k) => th[k] === 'การจอง');
    expect(key, 'การจอง is a value of th').toBeDefined();
    setLang('en');
    try {
      expect(menu.label()).toBe(en[key!]);
      expect(menu.label()).not.toMatch(THAI);
      expect(menu.heading.title()).not.toMatch(THAI);
      expect(menu.heading.subtitle()).not.toMatch(THAI);
    } finally {
      setLang('th');
    }
  });
});

describe('V2-bookings: the menu text table (W6, W10)', () => {
  it("holds v2's words verbatim in th", () => {
    const { th } = theTable();
    const values = new Set(Object.values(th));
    for (const word of V2_THAI) {
      expect(values.has(word), `th holds ${JSON.stringify(word)}`).toBe(true);
    }
  });

  it('gives en every key of th, in English, never empty', () => {
    const { th, en } = theTable();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [k, v] of Object.entries(en)) {
      expect(v.trim(), `en.${k} is not empty`).not.toBe('');
      expect(v, `en.${k} is English`).not.toMatch(THAI);
    }
  });

  it('is collected under the namespace admin.bookings and nowhere else', () => {
    const { th, en } = theTable();
    expect(() =>
      collectScreens({ '/src/screens/admin/bookings/i18n.ts': { th, en } }),
    ).not.toThrow();
    for (const k of Object.keys(th)) {
      expect(screenLookup.th[`admin.bookings.${k}`], `admin.bookings.${k}`).toBe(th[k]);
      expect(screenLookup.en[`admin.bookings.${k}`], `admin.bookings.${k}`).toBe(en[k]);
    }
  });
});
