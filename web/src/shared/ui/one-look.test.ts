// OL-1 (#6360): one look for every web page — the gate (docs/design/xchange-one-look.md §2.4).
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
//
// G1: no `.css` under web/src/ other than shared/ui/xc-theme.css holds a literal colour (hex, rgb(), rgba(), hsl(),
//     hsla(), or a CSS named colour as the value of a colour property), unless line 1 of the file is exactly the
//     marker `/* xc-look: legacy-colours */`. Exempt: screens/receipt-sample/receipt.css (byte-pinned fixture).
//     `transparent`, `currentColor`, `inherit` and `none` are not literals. Colours inside comments are ignored.
// G2: a file that carries the marker but has no literal colour left fails with "remove the marker" (ratchet).
// G3: AppShell and AuthLayout render `xc-theme` on their root element.
// G4: shared/ui/xc-theme.css exists, declares no custom property on `:root`, and defines at least the core
//     semantic properties under `.xc-theme`.
// G5: a file that carries the marker must be in the frozen list LEGACY_AT_OL1 (no new file can opt out).
// G0: the detector flags known literals and ignores comments and non-colour keywords; the CSS walk finds files.

import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { createComponent } from 'solid-js';
import { render } from 'solid-js/web';
import { AppShell } from './AppShell';
import { AuthLayout } from './AuthLayout';

interface NodeFs {
  readdirSync(dir: string): string[];
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
  statSync(file: string): { isDirectory(): boolean };
}

interface NodePath {
  resolve(...segments: string[]): string;
  join(...segments: string[]): string;
  dirname(p: string): string;
  relative(from: string, to: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';

let fs: NodeFs;
let path: NodePath;
let SRC_DIR: string;

const MARKER = '/* xc-look: legacy-colours */';
const THEME_FILE = 'shared/ui/xc-theme.css';
const EXEMPT = new Set<string>([THEME_FILE, 'screens/receipt-sample/receipt.css']);
const REQUIRED_THEME_PROPS = [
  '--xc-brand',
  '--xc-ink',
  '--xc-line',
  '--xc-surface',
  '--xc-focus-ring',
  '--xc-font-sans',
];

// G5: the files that carried the marker at the OL-1 skeleton. Frozen: never add to it. A wave removes a file's marker
// without editing this list (the check is marked ⊆ list), so waves never share this file.
const LEGACY_AT_OL1: ReadonlySet<string> = new Set<string>([
  'screens/admin/admin.css',
  'screens/admin/alert-settings/alert-settings.css',
  'screens/admin/booking-slots/booking-slots.css',
  'screens/admin/branches/branches.css',
  'screens/admin/campaign-performance/campaign-performance.css',
  'screens/admin/campaigns/campaigns.css',
  'screens/admin/cash/cash.css',
  'screens/admin/consent/consent.css',
  'screens/admin/customers/customers.css',
  'screens/admin/denomination-groups/denomination-groups.css',
  'screens/admin/executive/executive.css',
  'screens/admin/notices/notices.css',
  'screens/admin/overview/overview.css',
  'screens/admin/parts/formula.css',
  'screens/admin/parts/parts.css',
  'screens/admin/payments/payments.css',
  'screens/admin/pricing-formula/pricing-formula.css',
  'screens/admin/pricing/approve-results.css',
  'screens/admin/pricing/bulk-approve.css',
  'screens/admin/pricing/display-settings.css',
  'screens/admin/pricing/market-sources.css',
  'screens/admin/pricing/order-refs.css',
  'screens/admin/pricing/pricing-grid.css',
  'screens/admin/pricing/pricing-mode.css',
  'screens/admin/pricing/pricing-policy.css',
  'screens/admin/pricing/pricing-row.css',
  'screens/admin/pricing/pricing.css',
  'screens/admin/pricing/recommendations.css',
  'screens/admin/pricing/tier-editor.css',
  'screens/admin/queue/queue.css',
  'screens/admin/rate-alert-settings/rate-alert-settings.css',
  'screens/admin/rates/rates.css',
  'screens/admin/reconciliation/reconciliation.css',
  'screens/admin/reports/reports.css',
  'screens/admin/starting-prices/starting-prices.css',
  'screens/admin/transactions/transactions.css',
  'screens/admin/users/users.css',
  'screens/customer/_shell/xc-customer-tokens.css',
  'screens/customer/queue-display/queue-display.css',
  'screens/customer/rate-display/board/board-header.css',
  'screens/customer/rate-display/board/board-shell.css',
  'screens/customer/rate-display/board/board-stub.css',
  'screens/customer/rate-display/board/board-table.css',
  'screens/customer/rate-display/board/flap-price.css',
  'screens/customer/rate-display/board/route-strip.css',
  'screens/customer/rate-display/board/ticker.css',
  'screens/customer/rate-display/rate-display.css',
  'screens/desk-transaction/desk-transaction.css',
  'screens/desk/bookings/desk-bookings.css',
  'screens/desk/desk.css',
  'screens/desk/ocr/ocr.css',
  'screens/desk/referral/referral.css',
  'screens/desk/steps/refunds/refunds.css',
  'screens/devices/devices.css',
  'screens/documents/documents.css',
  'screens/kiosk-currencies/kiosk-currencies.css',
  'screens/operations/verification/verification.css',
  'screens/staff-guide/guide.css',
  'screens/staff-report/print.css',
  'screens/staff/attendance/attendance.css',
  'screens/staff/floor/floor.css',
  'screens/staff/holds/holds.css',
  'screens/staff/rates/rates.css',
  'screens/staff/shift/shift.css',
  'screens/staff/staff.css',
  'screens/staff/transfers/transfers.css',
  'shared/ui/auth.css',
  'shared/ui/card.css',
  'shared/ui/currency-count/currency-count.css',
  'shared/ui/notice-bell/notice-bell.css',
  'shared/ui/password-field.css',
  'shared/ui/permissions-unavailable.css',
  'shared/ui/queue-card.css',
  'shared/ui/queue-panel.css',
  'shared/ui/rate-freshness.css',
  'shared/ui/shell.css',
]);

// CSS Color 4 named colours. `transparent` and `currentColor` are deliberately absent.
const NAMED_COLOURS = new Set<string>(
  (
    'aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood ' +
    'cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod ' +
    'darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon ' +
    'darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray ' +
    'dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green ' +
    'greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon ' +
    'lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon ' +
    'lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen ' +
    'magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue ' +
    'mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy ' +
    'oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip ' +
    'peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown ' +
    'seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal ' +
    'thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen'
  ).split(' '),
);

// Design §2.4: a named colour counts only as the value of one of these properties.
const COLOUR_PROPERTY =
  /^(?:color|background|background-color|border(?:-[\w-]+)?|outline(?:-[\w-]+)?|fill|stroke|box-shadow|text-decoration-color)$/i;
const HEX = /#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![0-9a-zA-Z_-])/;
const COLOUR_FN = /(?<![\w-])(?:rgba?|hsla?)\(/i;

function walkCss(dir: string): string[] {
  let results: string[] = [];
  for (const entry of fs.readdirSync(dir)) {
    const full = path.join(dir, entry);
    if (fs.statSync(full).isDirectory()) {
      results = results.concat(walkCss(full));
    } else if (entry.endsWith('.css')) {
      results.push(full);
    }
  }
  return results;
}

/** Blank out comments, keeping newlines so offsets still map to the right line. */
function blankComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, ' '));
}

/** Every literal colour in a stylesheet, as `line: property: value`. */
function literalColours(css: string): string[] {
  const text = blankComments(css);
  const found: string[] = [];
  // A declaration ends at `;` or `}`; a selector or an at-rule prelude ends at `{`, so it never matches.
  const declaration = /([-\w]+)\s*:\s*([^;{}]*)(?=[;}])/g;
  let m: RegExpExecArray | null;
  while ((m = declaration.exec(text)) !== null) {
    const property = m[1];
    const value = m[2];
    const words = value.split(/[^\w-]+/).map((w) => w.toLowerCase());
    const literal =
      HEX.test(value) ||
      COLOUR_FN.test(value) ||
      (COLOUR_PROPERTY.test(property) && words.some((w) => NAMED_COLOURS.has(w)));
    if (literal) {
      const line = text.slice(0, m.index + m[0].indexOf(property)).split('\n').length;
      found.push(`${line}: ${property}: ${value.trim()}`);
    }
  }
  return found;
}

function hasMarker(css: string): boolean {
  return css.split('\n')[0]?.replace(/\r$/, '') === MARKER;
}

function sources(): { rel: string; css: string }[] {
  return walkCss(SRC_DIR)
    .map((file) => ({ rel: path.relative(SRC_DIR, file), css: fs.readFileSync(file, 'utf8') }))
    .filter((f) => !EXEMPT.has(f.rel));
}

let dispose: (() => void) | undefined;

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const thisDir = path.dirname(new URL(import.meta.url).pathname);
  SRC_DIR = path.resolve(thisDir, '../..');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

function mountRoot(build: () => ReturnType<typeof AppShell>): Element {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 503 })));
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(build, host);
  const root = host.firstElementChild;
  expect(root, 'the component renders a root element').not.toBeNull();
  return root!;
}

describe('one-look gate (OL-1, #6360, design §2.4)', () => {
  it('G0: the detector flags literal colours, ignores comments and keywords, and the CSS walk finds files', () => {
    expect(literalColours('.a { color: #a5003c; }'), '#a5003c').toHaveLength(1);
    expect(literalColours('.a { background: rgb(0 0 0); }'), 'rgb(0 0 0)').toHaveLength(1);
    expect(literalColours('.a { border-color: hsl(0 0% 0%); }'), 'hsl(0 0% 0%)').toHaveLength(1);
    expect(literalColours('.a { color: red; }'), 'color: red').toHaveLength(1);
    expect(literalColours('.a {\n  color: var(--xc-ink);\n  border: 1px solid #fff;\n}'), 'reports the line').toEqual([
      '3: border: 1px solid #fff',
    ]);
    expect(
      literalColours(
        '/* #a5003c rgb(0 0 0) color: red */\n.a { color: transparent; background: currentColor; ' +
          'border-color: inherit; outline: none; color: var(--xc-brand); }',
      ),
      'comments and transparent/currentColor/inherit/none are not literals',
    ).toEqual([]);
    expect(literalColours('#a5003c .a:hover { color: var(--xc-ink); }'), 'a selector is not a value').toEqual([]);
    expect(hasMarker(`${MARKER}\n.a { color: red; }`)).toBe(true);
    expect(hasMarker(`.a { color: red; }\n${MARKER}`)).toBe(false);
    expect(NAMED_COLOURS.size, 'the 148 CSS named colours').toBe(148);
    expect(walkCss(SRC_DIR).length, 'the CSS walk finds files under web/src').toBeGreaterThan(0);
  });

  it('G1: no literal colour in a .css under web/src/ outside xc-theme.css, unless the file carries the legacy marker', () => {
    const violations: string[] = [];
    for (const { rel, css } of sources()) {
      if (hasMarker(css)) continue;
      for (const hit of literalColours(css)) violations.push(`${rel}:${hit}`);
    }
    expect(
      violations,
      `Literal colours outside ${THEME_FILE}. Read a var(--xc-…) from xc-theme.css instead ` +
        `(design §2.1), or, for a file not yet swept, put "${MARKER}" on line 1:\n${violations.join('\n')}`,
    ).toEqual([]);
  });

  it('G2: a file with the legacy marker still has a literal colour (remove the marker once it has none)', () => {
    const stale: string[] = [];
    for (const { rel, css } of sources()) {
      if (hasMarker(css) && literalColours(css).length === 0) stale.push(rel);
    }
    expect(
      stale,
      `These files carry "${MARKER}" but have no literal colour left; remove the marker:\n${stale.join('\n')}`,
    ).toEqual([]);
  });

  it('G3: AppShell and AuthLayout render xc-theme on their root', () => {
    const shell = mountRoot(() =>
      createComponent(AppShell, { title: 'T', staffSession: false, children: 'body' }),
    );
    expect(shell.classList.contains('xc-shell'), 'AppShell root keeps xc-shell').toBe(true);
    expect(shell.classList.contains('xc-theme'), 'AppShell root carries xc-theme (design §2.2)').toBe(true);
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';

    const auth = mountRoot(() => createComponent(AuthLayout, { title: 'T', children: 'body' }));
    expect(auth.classList.contains('xc-auth'), 'AuthLayout root keeps xc-auth').toBe(true);
    expect(auth.classList.contains('xc-theme'), 'AuthLayout root carries xc-theme (design §2.2)').toBe(true);
  });

  it('G4: xc-theme.css exists, declares nothing on :root, and defines the core properties under .xc-theme', () => {
    const file = path.join(SRC_DIR, THEME_FILE);
    expect(fs.existsSync(file), `${THEME_FILE} must exist (design §2.1)`).toBe(true);
    const text = blankComments(fs.readFileSync(file, 'utf8'));

    expect(
      text.match(/:root\b[^{]*\{[^}]*--[\w-]+\s*:/g) ?? [],
      `${THEME_FILE} must not declare custom properties on :root (look-gate rule 2)`,
    ).toEqual([]);

    const defined = new Set<string>();
    const block = /([^{}]*)\{([^}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = block.exec(text)) !== null) {
      const selectors = m[1].split(',').map((s) => s.trim());
      if (!selectors.includes('.xc-theme')) continue;
      for (const d of m[2].matchAll(/(--[\w-]+)\s*:/g)) defined.add(d[1]);
    }
    const missing = REQUIRED_THEME_PROPS.filter((p) => !defined.has(p));
    expect(missing, `${THEME_FILE} must define these under .xc-theme (design §2.1)`).toEqual([]);
  });

  it('G5: every file with the legacy marker is in the frozen list LEGACY_AT_OL1', () => {
    const outside = sources()
      .filter(({ css }) => hasMarker(css))
      .map(({ rel }) => rel)
      .filter((rel) => !LEGACY_AT_OL1.has(rel));
    expect(
      outside,
      `Only files listed in LEGACY_AT_OL1 may carry "${MARKER}"; read var(--xc-…) from xc-theme.css instead:\n${outside.join('\n')}`,
    ).toEqual([]);
  });
});
