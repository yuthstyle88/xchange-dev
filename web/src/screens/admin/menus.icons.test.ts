// OL-6 B admin-sidebar-icons (audit Admin A row "sidebar icons mix colour emoji with mono glyphs; ⚖ ▣ 🏢 👥 📢 each
// reused for two menus"): every admin menu's sidebar icon is a single-colour text glyph, and no icon is used twice,
// counting the sidebar's four fixed links (AdminShell.tsx: queue ≡, kiosk ↗, devices ⎙, guide ?). The served v2 glyphs
// stay (menus.test.ts SERVED); payments moves off cash's ▣ to ▭ (menus.test.ts and Payments.test.tsx restated).
// Written before the implementation by claude/88867fb6 for the sub-controller claude/1cc51615. Do not edit, delete or
// weaken; add tests in a file of your own.
import { describe, expect, it } from 'vitest';
import { menus } from './menus';

/** The one icon of each menu, by menu id (= its folder under screens/admin). */
const ICONS: Record<string, string> = {
  overview: '▦',
  transactions: '⇄',
  cash: '▣',
  rates: '◎',
  payments: '▭',
  reports: '▥',
  customers: '♙',
  slips: '▧',
  approvals: '✓',
  'pricing-formula': 'ƒ',
  pricing: '▤',
  'alert-settings': '△',
  'rate-alert-settings': '↯',
  'booking-slots': '▩',
  branches: '⌂',
  executive: '♔',
  'campaign-performance': '◔',
  campaigns: '✦',
  notices: '!',
  'cash-limits': '≤',
  'cash-policy': '§',
  consent: '✎',
  'denomination-groups': '฿',
  loyalty: '☆',
  members: '♗',
  users: '♖',
  reconciliation: '=',
  'starting-prices': '◈',
};

/** The sidebar's fixed links (AdminShell.tsx; pinned by AdminShell.test.tsx "the tool links"). */
const FIXED = ['≡', '↗', '⎙', '?'];

describe('admin sidebar icons: one single-colour glyph set, none used twice', () => {
  it('SI-1 every menu carries the icon of the table, and the table names every menu', () => {
    const ids = menus.map((m) => m.id).sort();
    expect(ids).toEqual(Object.keys(ICONS).sort());
    for (const m of menus) expect(m.icon, `${m.id}.icon`).toBe(ICONS[m.id]);
  });

  it('SI-2 no icon is used twice, among the menus and the four fixed links', () => {
    const all = [...menus.map((m) => m.icon), ...FIXED];
    const seen = new Map<string, string[]>();
    menus.forEach((m) => seen.set(m.icon, [...(seen.get(m.icon) ?? []), m.id]));
    FIXED.forEach((g) => seen.set(g, [...(seen.get(g) ?? []), `fixed ${g}`]));
    const twice = [...seen].filter(([, who]) => who.length > 1);
    expect(twice, 'icons used more than once').toEqual([]);
    expect(new Set(all).size).toBe(all.length);
  });

  it('SI-3 each icon is one code point and not an emoji (no colour picture, no variation selector)', () => {
    for (const m of menus) {
      expect([...m.icon], `${m.id}.icon is one code point`).toHaveLength(1);
      expect(/\p{Emoji}/u.test(m.icon), `${m.id}.icon ${m.icon} is an emoji`).toBe(false);
      expect(/\p{Extended_Pictographic}/u.test(m.icon), `${m.id}.icon ${m.icon} is pictographic`).toBe(false);
    }
  });
});
