// OL-6 B admin-users (audit #564 issuecomment-6702970101, shots 042-045). users.css rules lose to admin.css's
// `.admin-main th/td` and its button rule, and the search submit inherits the theme's white submit text.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let readHere: (name: string) => string = () => '';

function stripped(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = stripped(css);
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    return out;
  }
  return undefined;
}

function media(css: string, query: string): string {
  const src = stripped(css);
  const at = src.indexOf(`@media (${query})`);
  if (at < 0) return '';
  const open = src.indexOf('{', at);
  let depth = 1;
  let i = open + 1;
  while (i < src.length && depth > 0) {
    if (src[i] === '{') depth += 1;
    if (src[i] === '}') depth -= 1;
    i += 1;
  }
  return src.slice(open + 1, i - 1);
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  readHere = (name) => fs.readFileSync(here + name, 'utf8');
});

const P = '.xc-admin .xc-content.xc-content';

describe('users look', () => {
  it('U-1 tabs: pill, muted when inactive, brand-filled when selected', () => {
    const css = readHere('users.css');
    const off = decls(css, `${P} .users-tab`);
    expect(off?.get('border-radius')).toBe('var(--xc-radius-pill)');
    expect(off?.get('color')).toBe('var(--xc-muted)');
    const on = decls(css, `${P} .users-tab[aria-selected="true"]`);
    expect(on?.get('background')).toBe('var(--xc-brand)');
    expect(on?.get('border-color')).toBe('var(--xc-brand)');
    expect(on?.get('color')).toBe('var(--xc-on-brand)');
    const tsx = readHere('UsersScreen.tsx');
    expect(tsx).toContain('class="users-btn users-tab"');
    expect(tsx).not.toContain("'users-btn users-btn-secondary' : 'users-btn'");
  });

  it('U-2 .users-btn names its text colour, so the submit button is not white on white', () => {
    expect(decls(readHere('users.css'), '.users-btn')?.get('color')).toBe('var(--xc-brand-text)');
  });

  it('U-3 toolbar: search 280px wide, 16px above the table card, action groups wrap', () => {
    const css = readHere('users.css');
    expect(decls(css, `${P} .users-toolbar input[type="search"]`)?.get('width')).toBe('280px');
    expect(decls(css, `${P} .users-toolbar input[type="search"]`)?.get('max-width')).toBe('100%');
    expect(decls(css, '.users-toolbar')?.get('margin-bottom')).toBe('16px');
    expect(decls(css, '.users-actions')?.get('flex-wrap')).toBe('wrap');
  });

  it('U-4 cell padding beats admin.css', () => {
    const css = readHere('users.css');
    expect(decls(css, '.xc-admin .xc-content .admin-main .users-th')?.get('padding')).toBe('10px 12px');
    expect(decls(css, '.xc-admin .xc-content .admin-main .users-td')?.get('padding')).toBe('12px');
  });

  it('U-5 above 800px row buttons are compact; at 800px or less they keep 44px', () => {
    const css = readHere('users.css');
    const wide = media(css, 'min-width: 801px');
    expect(wide, '@media (min-width: 801px)').not.toBe('');
    const btn = decls(wide, `${P} .users-row-actions button`);
    expect(btn?.get('min-height')).toBe('36px');
    expect(btn?.get('padding')).toBe('6px 12px');
    expect(btn?.get('font-size')).toBe('13px');
    expect(decls(wide, `${P} .users-role-tag .users-btn-revoke`)?.get('min-height')).toBe('28px');
    expect(decls(wide, '.users-row-actions')?.get('min-width')).toBe('268px');
    expect(decls(stripped(css).replace(wide, ''), `${P} .users-row-actions button`), 'no compact rule outside the media query').toBeUndefined();
  });

  it('U-6 every table card uses the shared card, with 20px padding', () => {
    const css = readHere('users.css');
    expect(decls(css, '.xc-admin .xc-content .users-card')?.get('padding')).toBe('20px');
    expect(decls(css, '.users-card'), 'the old copy of the card values is gone').toBeUndefined();
    expect(readHere('UsersScreen.tsx')).toContain("import '../../../shared/ui/card.css';");
    expect(readHere('UsersScreen.tsx')).toContain('<div class="users-card card">');
    expect(readHere('PermissionMatrix.tsx')).toContain('<div class="users-card card">');
    expect(readHere('UserAudit.tsx')).toContain('<div class="users-card card">');
    expect(readHere('LoginHistory.tsx')).toContain('<div class="users-card card">');
    expect(readHere('LoginHistory.tsx')).not.toContain('users-card users-table-scroll');
  });

  it('U-7 a table wider than its card shows a scroll cue at the right edge', () => {
    const bg = decls(readHere('users.css'), '.users-table-scroll')?.get('background') ?? '';
    expect(bg).toContain('no-repeat local');
    expect(bg).toContain('no-repeat scroll');
    expect(bg).toContain('var(--xc-line)');
  });

  it('U-8 the matrix panels are sections, not a card inside a card', () => {
    const d = decls(readHere('users.css'), '.users-matrix-panel');
    expect(d?.get('border-top')).toBe('1px solid var(--xc-line)');
    expect(d?.has('border')).toBe(false);
    expect(d?.has('background')).toBe(false);
    expect(d?.has('border-radius')).toBe(false);
  });

  it('U-9 before/after JSON wraps inside its column', () => {
    const d = decls(readHere('users.css'), '.users-table code');
    expect(d?.get('word-break')).toBe('break-all');
    expect(d?.get('font-size')).toBe('12px');
  });

  it('U-10 the pager row has its own rule', () => {
    expect(decls(readHere('users.css'), '.users-pager')?.get('justify-content')).toBe('flex-end');
  });
});
