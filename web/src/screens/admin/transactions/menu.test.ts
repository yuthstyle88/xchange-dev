// V2-transactions (#464): the menu record (ticket §7 `menu.ts`; design §12.3 in PR 474).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// The row of the served 10-06 nav (xchange-v2-source@4d0bdbb): `admin-workspace.js:11-20` group
// `desk`, label `รายการแลกเงิน`, icon `⇄`, second in the nav; `dated` (`admin-workspace.js:146`);
// title / subtitle `admin-workspace.js:89`; eyebrow v2's `pages.transactions` (`admin-html:73`);
// `roles: 'all'` (design §12.3). V2-shell's `menus.test.ts` holds every merged menu to the same
// table, but passes while this folder does not exist; this file fails until it does.
//
// `menu.ts` is read through an eager glob, so on a branch without it each test fails on the
// assertion `menu.ts exists` instead of the whole file failing to load.
import { describe, expect, it } from 'vitest';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { menus } from '../menus';

const loaded = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });

function theMenu(): AdminMenu {
  const m = loaded['./menu.ts'];
  expect(m, 'web/src/screens/admin/transactions/menu.ts exists').toBeDefined();
  expect(m!.menu, 'menu.ts exports `menu`').toBeDefined();
  return m!.menu;
}

describe('V2-transactions: the menu record (#464 §7)', () => {
  it("is the served nav's second desk menu: id transactions, ⇄, dated, every role", () => {
    const m = theMenu();
    expect(m.id).toBe('transactions');
    expect(m.group).toBe('desk');
    expect(m.order).toBe(2);
    expect(m.icon).toBe('⇄');
    expect(m.dated).toBe(true);
    expect(m.roles).toBe('all');
    expect(typeof m.Component).toBe('function');
  });

  it("carries v2's label and heading, in Thai through its own table", () => {
    setLang('th');
    const m = theMenu();
    expect(m.label()).toBe('รายการแลกเงิน');
    expect(m.heading.eyebrow()).toBe('TRANSACTIONS');
    expect(m.heading.title()).toBe('รายการแลกเงิน');
    expect(m.heading.subtitle()).toBe('ค้นหารายการ เปิดรายละเอียด ใบเสร็จ และเอกสารที่ผูกกับออเดอร์');
  });

  it('switches its label and heading with the language (W10): English, not Thai', () => {
    const m = theMenu();
    setLang('en');
    try {
      for (const [what, text] of [
        ['label', m.label()],
        ['title', m.heading.title()],
        ['subtitle', m.heading.subtitle()],
      ] as const) {
        expect(text, what).not.toBe('');
        expect(text, `${what} in English`).not.toMatch(/[฀-๿]/);
      }
    } finally {
      setLang('th');
    }
  });

  it("reaches the shell through menus.ts's glob, with no shell file edited (done criterion 2)", () => {
    const m = theMenu();
    const listed = menus.find((x) => x.id === 'transactions');
    expect(listed, 'menus lists transactions').toBeDefined();
    expect(listed).toBe(m);
  });
});
