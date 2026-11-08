// OL-8 reuse-customer-title-card (#564, round 13 "งาน css รียูส ด้วยนะ"): the customer page title and the
// customer form card were the same declaration block copied into several screen stylesheets. They now live
// once in shared/ui/customer-page.css (.xc-customer-page-title, .xc-customer-card); each screen uses the
// shared class and its copied rule is gone. Computed styles are unchanged: same declarations, still unlayered,
// so they keep beating the customer base layer (_shell/xc-customer-tokens.css @layer xc-customer-base).

import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}

// The shared rules read the xc-theme tokens (owner's round-13 order "งาน css รียูส ด้วยนะ": no hard-coded size
// where a token exists). --xc-space-1 = 4px = 0.25rem, --xc-text-4xl = 24px = 1.5rem, --xc-space-4 = 16px = 1rem,
// --xc-space-5 = 20px = 1.25rem: the computed look is unchanged. Restated by the sub-controller.
const TITLE = new Map([
  ['margin', '0 0 var(--xc-space-1)'],
  ['font-size', 'var(--xc-text-4xl)'],
  ['font-weight', '700'],
  ['color', 'var(--xc-text)'],
]);

const CARD = new Map([
  ['display', 'flex'],
  ['flex-direction', 'column'],
  ['gap', 'var(--xc-space-4)'],
  ['padding', 'var(--xc-space-5)'],
  ['background', 'var(--xc-surface)'],
  ['border', '0'],
  ['border-radius', 'var(--xc-radius-card)'],
  ['box-shadow', 'var(--xc-shadow-card)'],
]);

/** The copied blocks as they were written on the screens (rem literals); none may remain. */
const TITLE_COPY = new Map([...TITLE, ['margin', '0 0 0.25rem'], ['font-size', '1.5rem']]);
const CARD_COPY = new Map([...CARD, ['gap', '1rem'], ['padding', '1.25rem']]);


// Screen stylesheets that held a copy, and the copied selector each must no longer have.
const COPIES: Array<{ css: string; selector: string }> = [
  { css: 'book/book.css', selector: '.xc-book-title' },
  { css: 'bookings/bookings.css', selector: '.xc-bookings-title' },
  { css: 'branches/branches.css', selector: '.xc-branches-title-group h1' },
  { css: 'notifications/Notifications.css', selector: '.xc-notifications-title-group h1' },
  { css: 'rate-alerts/rate-alerts.css', selector: '.xc-alerts-title-group h1' },
  { css: 'bookings/bookings.css', selector: '.xc-booking-card' },
  { css: 'profile/profile.css', selector: '.xc-profile-form' },
  { css: 'queue/queue.css', selector: '.xc-queue-form' },
];

// The element in each screen that must carry the shared class, as its exact static class attribute.
const USERS: Array<{ tsx: string; markup: RegExp }> = [
  { tsx: 'book/Book.tsx', markup: /<h1 class="xc-customer-page-title">\{t\('title'\)\}<\/h1>/ },
  { tsx: 'bookings/Bookings.tsx', markup: /<h1 class="xc-customer-page-title">\{t\('title'\)\}<\/h1>/ },
  { tsx: 'branches/BranchesScreen.tsx', markup: /<h1 class="xc-customer-page-title">\{t\('title'\)\}<\/h1>/ },
  { tsx: 'notifications/Notifications.tsx', markup: /<h1 class="xc-customer-page-title">\{t\('title'\)\}<\/h1>/ },
  { tsx: 'rate-alerts/RateAlerts.tsx', markup: /<h1 class="xc-customer-page-title">\{t\('title'\)\}<\/h1>/ },
  { tsx: 'bookings/Bookings.tsx', markup: /<div class="xc-customer-card" data-booking-id=\{booking\.id\}>/ },
  { tsx: 'profile/Profile.tsx', markup: /<form class="xc-customer-card xc-profile-form" onSubmit=\{handleSave\}>/ },
  { tsx: 'queue/RequestForm.tsx', markup: /<form class="xc-customer-card" onSubmit=\{handleSubmit\}>/ },
];

const IMPORT = "import '../../../shared/ui/customer-page.css';";

let fs: NodeFs;
let here: string;
let sharedCss: string;

function read(rel: string): string {
  return fs.readFileSync(here + rel, 'utf8');
}

function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

// Innermost blocks only: "selector { declarations }" with no nested braces.
function blocks(css: string): Array<{ selector: string; decls: Map<string, string> }> {
  const out: Array<{ selector: string; decls: Map<string, string> }> = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripComments(css))) !== null) {
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) decls.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    out.push({ selector: m[1]!.trim().replace(/\s+/g, ' '), decls });
  }
  return out;
}

function sameDecls(a: Map<string, string>, b: Map<string, string>): boolean {
  if (a.size !== b.size) return false;
  for (const [k, v] of a) if (b.get(k) !== v) return false;
  return true;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  sharedCss = fs.existsSync(here + 'customer-page.css') ? read('customer-page.css') : '';
});

describe('OL-8 reuse-customer-title-card: one shared customer title and card', () => {
  it('RC-1: shared/ui/customer-page.css has .xc-customer-page-title with exactly the title declarations', () => {
    const rule = blocks(sharedCss).find((b) => b.selector === '.xc-customer-page-title');
    expect(rule, '.xc-customer-page-title rule in shared/ui/customer-page.css').toBeDefined();
    expect(Object.fromEntries(rule!.decls)).toEqual(Object.fromEntries(TITLE));
  });

  it('RC-2: shared/ui/customer-page.css has .xc-customer-card with exactly the card declarations', () => {
    const rule = blocks(sharedCss).find((b) => b.selector === '.xc-customer-card');
    expect(rule, '.xc-customer-card rule in shared/ui/customer-page.css').toBeDefined();
    expect(Object.fromEntries(rule!.decls)).toEqual(Object.fromEntries(CARD));
  });

  it('RC-3: the shared rules are unlayered, so they still beat the customer base layer', () => {
    expect(sharedCss).not.toBe('');
    expect(stripComments(sharedCss)).not.toMatch(/@layer/);
  });

  for (const { css, selector } of COPIES) {
    it(`RC-4: screens/customer/${css} no longer has the copied rule "${selector} { … }"`, () => {
      const left = blocks(read('../../screens/customer/' + css)).filter((b) => b.selector === selector);
      expect(left, `${selector} in ${css}`).toEqual([]);
    });
  }

  it('RC-5: no customer screen stylesheet re-copies either block under another selector', () => {
    const files = [...new Set(COPIES.map((c) => c.css))];
    const copies: string[] = [];
    for (const css of files) {
      for (const b of blocks(read('../../screens/customer/' + css))) {
        if (sameDecls(b.decls, TITLE) || sameDecls(b.decls, CARD) || sameDecls(b.decls, TITLE_COPY) || sameDecls(b.decls, CARD_COPY)) copies.push(`${css}: ${b.selector}`);
      }
    }
    expect(copies).toEqual([]);
  });

  for (const { tsx, markup } of USERS) {
    it(`RC-6: screens/customer/${tsx} uses the shared class (${markup.source.slice(0, 40)}…)`, () => {
      expect(read('../../screens/customer/' + tsx)).toMatch(markup);
    });
  }

  for (const tsx of [...new Set(USERS.map((u) => u.tsx))]) {
    it(`RC-7: screens/customer/${tsx} imports shared/ui/customer-page.css`, () => {
      expect(read('../../screens/customer/' + tsx).split('\n')).toContain(IMPORT);
    });
  }

  it('RC-8: the three title-group wrappers lost their rule-less class', () => {
    expect(read('../../screens/customer/branches/BranchesScreen.tsx')).not.toMatch(/xc-branches-title-group/);
    expect(read('../../screens/customer/notifications/Notifications.tsx')).not.toMatch(/xc-notifications-title-group/);
    expect(read('../../screens/customer/rate-alerts/RateAlerts.tsx')).not.toMatch(/xc-alerts-title-group/);
  });
});
