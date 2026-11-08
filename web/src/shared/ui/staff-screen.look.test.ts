// OL-8 reuse-staff-titles-buttons (round 13 CSS-reuse sweep, owner "งาน css รียูส ด้วยนะ"). Two blocks were copied
// across screen files on main 7a22b353: the small white bordered button (desk/referral, staff/attendance, staff/rates,
// staff/shift) and the staff card title (staff/attendance, staff/holds, staff/rates, staff/shift). They become one
// shared rule each in shared/ui/staff-screen.css; each screen keeps only the declarations that differ (its hover or
// disabled rule). staff/rates' table base moves to the shared table class of the list-states ticket. No visible
// change. Written before the fix by claude/2fee38 for the sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
  existsSync(file: string): boolean;
}

async function nodeFs(): Promise<NodeFs> {
  const specifier = 'node:fs';
  return (await import(/* @vite-ignore */ specifier)) as NodeFs;
}

/** This test's folder (web/src/shared/ui/). */
const HERE = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
const SRC = HERE + '../../';

async function readSrc(file: string): Promise<string> {
  return (await nodeFs()).readFileSync(SRC + file, 'utf8');
}

/** The declarations of the first rule whose selector list contains `selector`. */
function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

/** Every static class="..." value in a .tsx file, as token lists. */
function classLists(tsx: string): string[][] {
  const out: string[][] = [];
  const re = /\bclass="([^"{}\n]+)"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(tsx)) !== null) out.push(m[1]!.split(/\s+/).filter(Boolean));
  return out;
}

const SHARED = 'shared/ui/staff-screen.css';
const SHARED_IMPORT = "import '../../../shared/ui/staff-screen.css';";

// The shared table class that the list-states ticket (claude-2, slug reuse-admin-list-states) adds to
// shared/ui/list-states.css. If that ticket names it differently, the sub-controller restates this one constant.
const LIST_TABLE = 'xc-list-table';
const LIST_STATES_IMPORT = "import '../../../shared/ui/list-states.css';";

const BUTTONS = [
  { tsx: 'screens/desk/referral/ReferralCodeField.tsx', css: 'screens/desk/referral/referral.css', cls: 'desk-referral-btn', count: 1 },
  { tsx: 'screens/staff/attendance/AttendanceCard.tsx', css: 'screens/staff/attendance/attendance.css', cls: 'xc-attendance-btn', count: 4 },
  { tsx: 'screens/staff/rates/RatesCard.tsx', css: 'screens/staff/rates/rates.css', cls: 'xc-rates-retry-button', count: 1 },
  { tsx: 'screens/staff/shift/ShiftCard.tsx', css: 'screens/staff/shift/shift.css', cls: 'xc-shift-btn', count: 5 },
];

const TITLES = [
  { tsx: 'screens/staff/attendance/AttendanceCard.tsx', css: 'screens/staff/attendance/attendance.css', cls: 'xc-attendance-title' },
  { tsx: 'screens/staff/holds/HoldsCard.tsx', css: 'screens/staff/holds/holds.css', cls: 'xc-holds-title' },
  { tsx: 'screens/staff/rates/RatesCard.tsx', css: 'screens/staff/rates/rates.css', cls: 'xc-rates-title' },
  { tsx: 'screens/staff/shift/ShiftCard.tsx', css: 'screens/staff/shift/shift.css', cls: 'xc-shift-title' },
];

describe('staff-screen shared title and button (OL-8 reuse)', () => {
  it('RS-1 staff-screen.css holds the shared title rule, with tokens', async () => {
    expect((await nodeFs()).existsSync(SRC + SHARED), `${SHARED} must exist`).toBe(true);
    const d = decls(await readSrc(SHARED), '.xc-screen-title');
    expect(d && Object.fromEntries(d)).toEqual({
      margin: '0',
      'font-size': 'var(--xc-text-2xl)',
      'font-weight': '600',
      color: 'var(--xc-ink)',
    });
  });

  it('RS-2 staff-screen.css holds the shared small white bordered button, with tokens', async () => {
    const d = decls(await readSrc(SHARED), '.xc-screen-btn');
    expect(d && Object.fromEntries(d)).toEqual({
      padding: '6px var(--xc-space-3)',
      'font-size': 'var(--xc-text-base)',
      'font-weight': '500',
      color: 'var(--xc-ink)',
      background: 'var(--xc-surface)',
      border: '1px solid var(--xc-line)',
      'border-radius': 'var(--xc-radius-sm)',
      cursor: 'pointer',
    });
  });

  it('RS-3 no screen keeps its own copy of the title rule', async () => {
    for (const { css, cls } of TITLES) {
      expect(decls(await readSrc(css), `.${cls}`), `${css} must not keep .${cls}`).toBeUndefined();
    }
  });

  it('RS-4 every screen title is an h2 with the shared class only', async () => {
    for (const { tsx, cls } of TITLES) {
      const code = await readSrc(tsx);
      expect(code, `${tsx} keeps class ${cls}`).not.toMatch(new RegExp(`\\b${cls}(?![\\w-])`));
      expect(code, `${tsx} title`).toMatch(/<h2 class="xc-screen-title">/);
    }
  });

  it('RS-5 no screen keeps its own copy of the button base rule', async () => {
    for (const { css, cls } of BUTTONS) {
      expect(decls(await readSrc(css), `.${cls}`), `${css} must not keep the base .${cls} rule`).toBeUndefined();
    }
  });

  it('RS-6 each screen keeps only the button state that differs', async () => {
    expect(decls(await readSrc('screens/desk/referral/referral.css'), '.desk-referral-btn:disabled')?.get('opacity')).toBe('0.5');
    expect(decls(await readSrc('screens/staff/attendance/attendance.css'), '.xc-attendance-btn:hover')?.get('background')).toBe('var(--xc-paper)');
    expect(decls(await readSrc('screens/staff/rates/rates.css'), '.xc-rates-retry-button:hover')?.get('background')).toBe('var(--xc-gray-50)');
    const shift = await readSrc('screens/staff/shift/shift.css');
    expect(decls(shift, '.xc-shift-btn:hover')?.get('background')).toBe('var(--xc-gray-50)');
    // The danger variant must still beat the shared base whatever order the bundle puts the files in.
    const danger = decls(shift, '.xc-shift-btn.xc-shift-btn-danger');
    expect(danger?.get('color')).toBe('var(--xc-danger)');
    expect(danger?.get('border-color')).toBe('var(--xc-brand-200)');
    expect(decls(shift, '.xc-shift-btn-danger'), 'the bare .xc-shift-btn-danger rule becomes .xc-shift-btn.xc-shift-btn-danger').toBeUndefined();
  });

  it('RS-7 every button that had the screen class also carries the shared class, first', async () => {
    for (const { tsx, cls, count } of BUTTONS) {
      const lists = classLists(await readSrc(tsx)).filter((l) => l.includes(cls));
      expect(lists.length, `${tsx}: buttons with ${cls}`).toBe(count);
      for (const l of lists) expect(l[0], `${tsx}: class="${l.join(' ')}"`).toBe('xc-screen-btn');
    }
  });

  it('RS-8 each screen imports staff-screen.css just before its own stylesheet', async () => {
    const files = new Set([...BUTTONS, ...TITLES].map((f) => f.tsx));
    for (const tsx of files) {
      const code = await readSrc(tsx);
      const own = /import '\.\/[a-z-]+\.css';/.exec(code);
      expect(own, `${tsx} own css import`).not.toBeNull();
      expect(code.indexOf(SHARED_IMPORT), `${tsx} imports ${SHARED}`).toBeGreaterThanOrEqual(0);
      expect(code.indexOf(SHARED_IMPORT), `${tsx}: shared import before ${own![0]}`).toBeLessThan(own!.index);
    }
  });
});

describe('staff/rates table base uses the shared list table (OL-8 reuse, after list-states)', () => {
  it('RS-9 rates.css keeps no table base rule', async () => {
    expect(decls(await readSrc('screens/staff/rates/rates.css'), '.xc-rates-table')).toBeUndefined();
  });

  it('RS-10 the rates table carries the shared table class and RatesCard imports list-states.css first', async () => {
    const code = await readSrc('screens/staff/rates/RatesCard.tsx');
    expect(code).toMatch(new RegExp(`<table class="${LIST_TABLE}">`));
    expect(code).not.toMatch(/\bxc-rates-table\b(?!-)/);
    const listAt = code.indexOf(LIST_STATES_IMPORT);
    expect(listAt, 'RatesCard imports list-states.css').toBeGreaterThanOrEqual(0);
    expect(listAt, 'list-states.css before rates.css').toBeLessThan(code.indexOf("import './rates.css';"));
  });
});
