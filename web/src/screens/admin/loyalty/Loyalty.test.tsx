// LY-6-FE (#2129): up-front tests for the EXECUTIVE loyalty rule-version screen
// (design `docs/design/blueprint/membership-points.md` §4.6, §6.4, §9.1 row LY-6-FE).
//
// Written by the ticket's writer before the screen exists. The implementer makes them pass and must not edit,
// delete or weaken them. No threshold, ratio or tier name appears here (§3 [required]): the numbers are data the
// EXECUTIVE enters.
//
// The menu is loaded through `import.meta.glob`, so a missing `menu.ts` fails these tests on their result
// ("no loyalty menu"), not on an unresolved import.
//
// Rows 1–3 and the screen half of row 4 were `it.todo` here until the rule-version view (#2868, PR 2909) existed;
// they now live in Loyalty.screen.test.tsx.
import { describe, expect, it } from 'vitest';
import { errorText, setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';

const found = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
const loyaltyMenu = (): AdminMenu | undefined => Object.values(found)[0]?.menu;

describe('LY-6-FE row 5: the menu needs LoyaltyRulesManage (#2129 §9)', () => {
  it('row 5: the loyalty menu exists, is a Blueprint menu and needs LoyaltyRulesManage, so a user without it does not see it', () => {
    const m = loyaltyMenu();
    expect(m, 'web/src/screens/admin/loyalty/menu.ts exports `menu`').toBeDefined();
    // AdminShell shows a menu only when `needs === 'all'` or the user holds `needs` (AdminShell.tsx:281).
    expect(m?.needs, "the menu is not open to everyone ('all')").not.toBe('all');
    expect(m?.needs).toBe('LoyaltyRulesManage');
    expect(m?.blueprint, 'blueprint: true (§9.1 LY-6-FE)').toBe(true);
    expect(typeof m?.Component, 'the menu renders a component').toBe('function');
  });
});

describe('LY-6-FE row 4: 503 loyaltyUnavailable has its own words, not error.serverError (#2129 §9, design §6.4)', () => {
  for (const lang of ['th', 'en', 'zh'] as const) {
    it(`row 4 (${lang}): error.loyaltyUnavailable exists and differs from error.serverError`, () => {
      setLang(lang);
      const sentinel = 'raw server message';
      const worded = errorText('loyaltyUnavailable', sentinel);
      expect(worded, `error.loyaltyUnavailable is in ${lang}.ts`).not.toBe(sentinel);
      expect(worded.trim(), 'the text is not empty').not.toBe('');
      expect(worded, 'it is not the generic server error').not.toBe(errorText('serverError', sentinel));
    });
  }
  // The screen half of row 4 is in Loyalty.screen.test.tsx (#2129, claude/177aa7ef).
});

// Rows 1–3, the screen half of row 4, and rows 6–7 are real tests in Loyalty.screen.test.tsx, written against the
// rule-version view #2868 (PR 2909) once it gave the screen a read (#2129, claude/177aa7ef).
