// V2-shell (#460): the admin menu list built from each menu's folder (ticket §7 `menus.ts`, §9,
// design §3.3 and §12.3 in PR 474). Written before the implementation by the lane controller
// (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// `collectMenus` is pure, so it is driven with fixture globs: no real menu exists when this
// merges. The last block checks every menu that does exist (none yet) against the served 10-06
// nav, so each V2-<menu> ticket's folder is held to the row of its menu below.
//
// `menus.ts` is read through an eager glob, so on a branch without it each test fails on the
// assertion `menus.ts exists` instead of the whole file failing to load.
import { describe, expect, it } from 'vitest';
import type { JSX } from 'solid-js';
import { setLang } from '../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from './menus';

type MenusModule = typeof import('./menus');

const loaded = import.meta.glob<MenusModule>('./menus.ts', { eager: true });

function mod(): MenusModule {
  const m = loaded['./menus.ts'];
  expect(m, 'web/src/screens/admin/menus.ts exists').toBeDefined();
  return m!;
}

// ── the served nav (xchange-v2-source@4d0bdbb) ───────────────────────────────
//
// group, order, label, icon: src/devices-public/admin-workspace.js:11-20 (groups :11-16 in this
// order, labels :17-19, icons :20); `referrals` (:14) is not ported (owner: Broker-Platform).
// dated: admin-workspace.js:146. title / subtitle: admin-workspace.js:85-97 (`descriptions`).
// eyebrow: v2's `pages` entry of the menu — src/online/lib/admin-html.ts decoded line 73
// (overview, transactions, rates, reports) and :112 (cash); src/online/public/bookings.js:2,
// slips.js:2, employees.js:2, customers.js:2; src/devices-public/admin-payments.js:6. The
// workspace replaces only the title and the subtitle (admin-workspace.js:145), never the eyebrow.

interface Served {
  group: 'desk' | 'money' | 'data' | 'tools';
  order: number;
  label: string;
  icon: string;
  dated: boolean;
  eyebrow: string;
  title: string;
  subtitle: string;
}

const SERVED: Record<string, Served> = {
  overview: {
    group: 'desk',
    order: 1,
    label: 'ภาพรวม',
    icon: '▦',
    dated: true,
    eyebrow: 'DAILY OVERVIEW',
    title: 'ภาพรวมหน้าร้าน',
    subtitle: 'ติดตามสถานะกะและยอดรายการในช่วงวันที่ที่เลือก',
  },
  transactions: {
    group: 'desk',
    order: 2,
    label: 'รายการแลกเงิน',
    icon: '⇄',
    dated: true,
    eyebrow: 'TRANSACTIONS',
    title: 'รายการแลกเงิน',
    subtitle: 'ค้นหารายการ เปิดรายละเอียด ใบเสร็จ และเอกสารที่ผูกกับออเดอร์',
  },
  bookings: {
    group: 'desk',
    order: 3,
    label: 'การจอง',
    icon: '▤',
    dated: false,
    eyebrow: 'ONLINE BOOKINGS',
    title: 'การจอง',
    subtitle: 'ตรวจการจอง ล็อกเรท และส่งต่อไปทำรายการหน้าร้าน',
  },
  cash: {
    group: 'money',
    order: 4,
    label: 'เงินสดและกะ',
    icon: '▣',
    dated: false,
    eyebrow: 'CASH & SHIFTS',
    title: 'เงินสดและกะ',
    subtitle: 'เปิดกะ รับ–จ่าย ตรวจนับ และติดตามประวัติในสาขาเดียวกัน',
  },
  rates: {
    group: 'money',
    order: 5,
    label: 'เรทและสกุลเงิน',
    icon: '◎',
    dated: false,
    eyebrow: 'RATE MANAGEMENT',
    title: 'เรทและสกุลเงิน',
    subtitle: 'เรทหน้าร้านและข้อมูลอ้างอิงสำหรับสาขาที่เลือก',
  },
  payments: {
    group: 'money',
    order: 6,
    label: 'วิธีชำระเงิน',
    icon: '▣',
    dated: false,
    eyebrow: 'PAYMENT SETTINGS',
    title: 'ตั้งค่าวิธีชำระเงิน',
    subtitle: 'เปิด–ปิดช่องทาง กำหนดข้อความแนะนำ และดูประวัติการตั้งค่า',
  },
  reports: {
    group: 'money',
    order: 7,
    label: 'รายงาน',
    icon: '▥',
    dated: true,
    eyebrow: 'DAILY REPORT',
    title: 'รายงาน',
    subtitle: 'แยกดูช่องทางชำระเงิน สกุลเงิน และยอดของพนักงาน',
  },
  customers: {
    group: 'data',
    order: 8,
    label: 'ลูกค้าและสมาชิก',
    icon: '♙',
    dated: false,
    eyebrow: 'CUSTOMERS',
    title: 'ลูกค้าและสมาชิก',
    subtitle: 'ประวัติลูกค้าและคำขอลงทะเบียน แยกเป็นหมวดเพื่อค้นหาได้เร็ว',
  },
  slips: {
    group: 'data',
    order: 9,
    label: 'สลิปและหลักฐาน',
    icon: '▧',
    dated: false,
    eyebrow: 'PAYMENT SLIPS',
    title: 'สลิปและหลักฐาน',
    subtitle: 'ค้นหาหลักฐานรับ–จ่ายจากรายการแลกเงิน',
  },
  employees: {
    group: 'data',
    order: 10,
    label: 'พนักงานและสิทธิ์',
    icon: '♙',
    dated: false,
    eyebrow: 'TEAM & OPERATIONS',
    title: 'พนักงานและสิทธิ์',
    subtitle: 'บัญชีพนักงานและการตั้งค่าขั้นตอนรับ–ส่งเงิน',
  },
};

// ── fixture menus ────────────────────────────────────────────────────────────

function Nothing(_props: AdminMenuProps): JSX.Element {
  return null;
}

function fixture(id: string, order: number, over: Partial<AdminMenu> = {}): AdminMenu {
  return {
    id,
    label: () => `label ${id}`,
    icon: '▦',
    order,
    group: 'desk',
    roles: 'all',
    heading: { eyebrow: () => 'EYEBROW', title: () => `title ${id}`, subtitle: () => `subtitle ${id}` },
    dated: false,
    Component: Nothing,
    ...over,
  };
}

/** A glob as `import.meta.glob('./*\/menu.ts', { eager: true })` answers it: folder → `{ menu }`. */
function glob(...entries: [folder: string, menu: AdminMenu][]): Record<string, { menu: AdminMenu }> {
  const out: Record<string, { menu: AdminMenu }> = {};
  for (const [folder, menu] of entries) out[`./${folder}/menu.ts`] = { menu };
  return out;
}

describe('V2-shell: collectMenus (#460 §7, §9; design §3.3)', () => {
  it('sorts the menus by order, whatever order the glob lists them in, and keeps each menu as given', () => {
    const { collectMenus } = mod();
    const reports = fixture('reports', 7);
    const overview = fixture('overview', 1);
    const cash = fixture('cash', 4);
    const out = collectMenus(glob(['reports', reports], ['overview', overview], ['cash', cash]));
    expect(out.map((m) => m.id)).toEqual(['overview', 'cash', 'reports']);
    expect(out[0]).toBe(overview);
    expect(out[1]).toBe(cash);
    expect(out[2]).toBe(reports);
  });

  it('answers an empty list when no menu folder exists yet (no menu merged)', () => {
    const { collectMenus } = mod();
    expect(collectMenus({})).toEqual([]);
  });

  it('refuses two menus with the same order (design §3.3)', () => {
    const { collectMenus } = mod();
    expect(() => collectMenus(glob(['overview', fixture('overview', 1)], ['cash', fixture('cash', 1)]))).toThrow();
    // the same two with distinct orders are accepted, so the refusal is the order's
    expect(collectMenus(glob(['overview', fixture('overview', 1)], ['cash', fixture('cash', 4)]))).toHaveLength(2);
  });

  it('refuses two menus with the same id (design §3.3)', () => {
    const { collectMenus } = mod();
    expect(() =>
      collectMenus(glob(['overview', fixture('overview', 1)], ['overview-copy', fixture('overview', 2)])),
    ).toThrow();
  });

  it('refuses a menu whose id is not its folder name (/#<id> = folder = i18n namespace)', () => {
    const { collectMenus } = mod();
    expect(() => collectMenus(glob(['overview', fixture('dashboard', 1)]))).toThrow();
    expect(() => collectMenus(glob(['cash', fixture('cash', 4)], ['rates', fixture('rate', 5)]))).toThrow();
    expect(collectMenus(glob(['cash', fixture('cash', 4)], ['rates', fixture('rates', 5)]))).toHaveLength(2);
  });

  it("refuses the id 'parts' (V2-parts' folder is never a menu, design §12.2)", () => {
    const { collectMenus } = mod();
    expect(() => collectMenus(glob(['parts', fixture('parts', 11)]))).toThrow();
  });

  it('refuses an id outside [a-z][a-z0-9-]* even when it is its folder name (ticket §7 `id`)', () => {
    const { collectMenus } = mod();
    for (const bad of ['Overview', '1cash', 'cash_flow', '-rates', 'ภาพรวม']) {
      expect(() => collectMenus(glob([bad, fixture(bad, 1)])), bad).toThrow();
    }
    for (const good of ['cash', 'cash-flow', 'r2']) {
      expect(collectMenus(glob([good, fixture(good, 1)])), good).toHaveLength(1);
    }
  });
});

// Menus the Blueprint adds that v2 never served. Amended by #602 (RE-5), by its ticket author
// (claude/e5ab6c): A06 `/admin/approvals` (`docs/design/blueprint/rates.md` §4.7, §9 RE-5; Blueprint
// §03) is a v3 menu with no v2 row, OWNER only (its view and command are OWNER rows), so it can match
// neither SERVED nor `roles: 'all'`. Its own record is pinned in
// `web/src/screens/admin/approvals/Approvals.test.tsx`. Every served menu is still held to its row.
// Amended by #655 (CI-8), by its ticket author's delegate (claude/5edf7bb5): `/admin/consent`
// (`docs/design/blueprint/customer-identity-consent.md` §4.4, §9 CI-8) is a v3 menu with no v2 row,
// OWNER only (`consentPolicy` is an owner row, PR 876), pinned in
// `web/src/screens/admin/consent/Consent.test.tsx`.
const BLUEPRINT_MENUS: readonly string[] = ['approvals', 'consent'];

describe('V2-shell: every merged menu follows the served nav (#460 §7; design §12.3)', () => {
  it('lists the merged menu folders sorted by order, ids and orders unique, every one a served menu', () => {
    const { menus } = mod();
    expect(Array.isArray(menus)).toBe(true);
    const orders = menus.map((m) => m.order);
    expect(orders).toEqual([...orders].sort((a, b) => a - b));
    expect(new Set(menus.map((m) => m.id)).size).toBe(menus.length);
    expect(new Set(orders).size).toBe(menus.length);
    for (const m of menus) {
      if (BLUEPRINT_MENUS.includes(m.id)) continue;
      expect(Object.keys(SERVED), `${m.id} is a served menu (referrals is not ported)`).toContain(m.id);
    }
  });

  it("gives each merged menu its served group, order, icon, label, heading and period toolbar, and roles 'all'", () => {
    const { menus } = mod();
    setLang('th');
    for (const m of menus.filter((x) => !BLUEPRINT_MENUS.includes(x.id))) {
      const s = SERVED[m.id]!;
      expect(m.group, `${m.id}.group`).toBe(s.group);
      expect(m.order, `${m.id}.order`).toBe(s.order);
      expect(m.icon, `${m.id}.icon`).toBe(s.icon);
      expect(m.dated, `${m.id}.dated`).toBe(s.dated);
      expect(m.label(), `${m.id}.label`).toBe(s.label);
      expect(m.heading.eyebrow(), `${m.id}.heading.eyebrow`).toBe(s.eyebrow);
      expect(m.heading.title(), `${m.id}.heading.title`).toBe(s.title);
      expect(m.heading.subtitle(), `${m.id}.heading.subtitle`).toBe(s.subtitle);
      expect(m.roles, `${m.id}.roles`).toBe('all');
      expect(typeof m.Component, `${m.id}.Component`).toBe('function');
    }
  });
});
