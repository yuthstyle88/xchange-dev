// EX-4 (#1186): the A01 menu's record, its discovery through the shell's glob, its own TH/EN text table,
// and the rules on its source (ticket §6, §7; design `docs/design/blueprint/executive-finance.md` §4.2, §7.1,
// L-A4). Written before the implementation by the test-writer claude/5d45b017, on the FE lane controller's
// order (claude/5edf7bb5). Do not edit, delete or weaken; add tests in a file of your own.
//
// The menu is visible by **permission**, never by role (ticket §7 [required]): `needs: 'OverviewRead'`, the
// field OR-7.7 #1030 gives `AdminMenu` in place of `roles`. #1030 is not on this branch's base, so `needs` is
// read loosely from the module, and this file compiles both before and after it lands.
//
// The menu's files are read through eager globs, so on a branch without them each test fails on an
// assertion that the file exists, not on the import.
import { describe, expect, it } from 'vitest';
import type { JSX } from 'solid-js';
import { screenLookup, setLang } from '../../../shared/i18n';
import { menus, type AdminMenu, type AdminMenuProps } from '../menus';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
interface TextsModule {
  th?: Record<string, string>;
  en?: Record<string, string>;
  zh?: Record<string, string>;
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });
const textFiles = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });
const sources = import.meta.glob<string>('./*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
});
const styles = import.meta.glob<string>('./*.css', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function menuModule(): MenuModule {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/executive/menu.ts exists').toBeDefined();
  return m!;
}

function texts(): { th: Record<string, string>; en: Record<string, string>; module: TextsModule } {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/executive/i18n.ts exists').toBeDefined();
  expect(m!.th, 'i18n.ts exports a th table').toBeDefined();
  expect(m!.en, 'i18n.ts exports an en table').toBeDefined();
  return { th: m!.th!, en: m!.en!, module: m! };
}

/** Whitespace does not count when matching a text: the wireframe writes both `จอง→สำเร็จ` and `จอง → สำเร็จ`. */
function squash(s: string): string {
  return s.replace(/\s+/g, '');
}

/**
 * A01's own Thai texts (ticket §8; design §3, §4.2–§4.4): the six KPI labels of E-1, the header, the branch
 * table's columns, the alert list and the three alert kinds EX-3 sends (#1071 ruling, gap 6), and the
 * drill-down's dropped-filter note.
 */
const TEXTS = [
  // header (§4.2)
  'ทุกสาขา',
  'ข้อมูลล่าสุด',
  // the six KPI tiles (E-1, design §3)
  'ยอดซื้อ',
  'ยอดขาย',
  'ส่วนต่างรับรู้',
  'ลูกค้าซ้ำ',
  'จอง → สำเร็จ',
  'ค่าคอม',
  // the branch table (§4.2)
  'สาขา',
  'ยอด',
  'เงินคงเหลือ',
  'ปิดกะ',
  'เวลา sync',
  // the alerts (§4.3; wire kinds ratePending / staleData / countDifference)
  'ต้องดำเนินการ',
  'ข้อมูลค้าง',
  'เรทรออนุมัติ',
  'เงินนับไม่ตรง',
  // drill-down (§4.4)
  'ตัวกรองนี้ยังไม่ส่งต่อ',
] as const;

/** Role names: none may be written in the folder's source (ticket §7, the lane controller's order). */
const ROLE_LITERAL =
  /['"`](EXECUTIVE|ADMIN|OWNER|TELLER|FINANCE|BRANCH_MANAGER|RATE_MANAGER|MARKETING|VERIFICATION_REVIEWER|FRONT|BACKEND|MANAGER|RATES|SYSTEM|MONITOR)['"`]/;

describe('EX-4 A01: the menu record (#1186 §7)', () => {
  it("is id 'executive', group 'data', dated, with a label and a heading in both languages", () => {
    const { menu } = menuModule();
    setLang('th');
    expect(menu.id).toBe('executive');
    expect(menu.group).toBe('data');
    expect(menu.dated, 'A01 takes the period from the shell').toBe(true);
    expect(typeof menu.order).toBe('number');
    expect(typeof menu.icon).toBe('string');
    expect(menu.icon.trim()).not.toBe('');
    expect(menu.label().trim()).not.toBe('');
    expect(menu.heading.title().trim()).not.toBe('');
    expect(typeof menu.Component).toBe('function');
  });

  it("is shown by the permission OverviewRead (`needs`, #1030 §3.10), with no `roles` list", () => {
    const { menu } = menuModule();
    const loose = menu as unknown as Record<string, unknown>;
    expect(loose.needs, "menu.needs is the permission 'OverviewRead'").toBe('OverviewRead');
    expect(loose.roles, 'no role list: visibility is by permission').toBeUndefined();
  });

  it("reaches the shell through menus.ts's glob (./*/menu.ts) once, with no shell file edited", () => {
    const { menu } = menuModule();
    const listed = menus.filter((m) => m.id === 'executive');
    expect(listed.length, 'executive is listed once by ./*/menu.ts').toBe(1);
    expect(listed[0]).toBe(menu);
    const others = menus.filter((m) => m.id !== 'executive').map((m) => m.order);
    expect(others, 'its order is free').not.toContain(menu.order);
  });

  it('follows setLang: label and heading switch to the en table', () => {
    const { menu } = menuModule();
    const { th, en } = texts();
    try {
      setLang('th');
      const thLabel = menu.label();
      const thTitle = menu.heading.title();
      const keyOf = (text: string): string => {
        const found = Object.entries(th).find(([, v]) => v === text);
        expect(found, `${text} is a value of the menu's th table`).toBeDefined();
        return found![0];
      };
      setLang('en');
      expect(menu.label()).toBe(en[keyOf(thLabel)]);
      expect(menu.heading.title()).toBe(en[keyOf(thTitle)]);
    } finally {
      setLang('th');
    }
  });
});

describe("EX-4 A01: the menu's own text table (L-A4: TH/EN)", () => {
  it('holds every A01 text in th, with an English twin for every key and no zh table', () => {
    const { th, en, module } = texts();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    expect(module.zh, 'a back-office screen has no zh table (L-A4)').toBeUndefined();
    const values = Object.values(th).map(squash);
    for (const text of TEXTS) {
      expect(values, `the menu's th table holds ${JSON.stringify(text)}`).toContain(squash(text));
    }
    for (const [key, value] of Object.entries(en)) {
      expect(value.trim(), `en.${key} is not empty`).not.toBe('');
    }
  });

  it('is collected under the namespace admin.executive.* and adds no shared key', () => {
    const { th, en } = texts();
    for (const key of Object.keys(th)) {
      expect(screenLookup.th[`admin.executive.${key}`], `admin.executive.${key} (th)`).toBe(th[key]);
      expect(screenLookup.en[`admin.executive.${key}`], `admin.executive.${key} (en)`).toBe(en[key]);
    }
  });
});

describe('EX-4 A01: the source (ticket §7 [required]; the lane controller’s rules)', () => {
  const own = Object.entries(sources).filter(([path]) => !path.includes('.test.'));

  it('has its source files in this folder', () => {
    const paths = own.map(([p]) => p);
    expect(paths).toContain('./menu.ts');
    expect(paths).toContain('./i18n.ts');
  });

  it('never parses, rounds or scales a figure: money and counts stay strings', () => {
    const forbidden = /Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/;
    for (const [path, text] of own) {
      text.split('\n').forEach((line, i) => {
        expect(forbidden.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
      });
    }
  });

  it('names no role anywhere: visibility and behaviour are by permission', () => {
    for (const [path, text] of own) {
      text.split('\n').forEach((line, i) => {
        expect(ROLE_LITERAL.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
      });
    }
  });

  it('has its own stylesheet, executive.css, imported by the screen and declaring nothing on :root', () => {
    const css = styles['./executive.css'];
    expect(css, 'web/src/screens/admin/executive/executive.css exists').toBeDefined();
    expect(/:root/.test(css!), 'no custom properties on :root (LK-6)').toBe(false);
    const importers = own.filter(([, text]) => /import\s+['"]\.\/executive\.css['"]/.test(text));
    expect(importers.length, 'a source file imports ./executive.css').toBeGreaterThan(0);
  });
});
