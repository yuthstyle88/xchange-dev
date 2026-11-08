// OL-6 wave B staff-guide (audit #564 issuecomment-6702970101, shots 014-015): the page undoes the shell (flat header,
// 880px body column, reverted print button, no footer) and its sections are hairlines. print.test.tsx pins v2's `body`,
// `h1,h2`, `section` and `a` rule text and StaffGuide.test.tsx pins the "กลับแอดมิน" text, so those stay.
// Written before the fix by claude/bbe9ce8b. Do not edit, delete or weaken.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { StaffGuide } from './StaffGuide';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

async function readHere(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8');
}

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

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('staff-guide look', () => {
  it('SG-1 no rule undoes the shared shell any more', async () => {
    const css = await readHere('guide.css');
    for (const sel of [
      '.xc-shell .xc-shell-header',
      '.xc-shell .xc-shell-nav',
      '.xc-shell .xc-shell-title',
      '.xc-shell .xc-shell-nav a',
      '.xc-shell .xc-shell-nav a:focus-visible',
      '.xc-shell .xc-shell-nav button',
      '.xc-shell .xc-shell-main',
      '.xc-shell .xc-shell-footer',
    ]) {
      expect(decls(css, sel), `${sel} removed`).toBeUndefined();
    }
    expect(css).not.toMatch(/all\s*:\s*revert/);
  });

  it('SG-2 the page body has no column or margin once the shell is there', async () => {
    const d = decls(await readHere('guide.css'), 'html body:has(.xc-shell)');
    expect(d?.get('max-width')).toBe('none');
    expect(d?.get('margin')).toBe('0');
    expect(d?.get('padding')).toBe('0');
  });

  // Restated 2026-11-07 by claude/studio-claude-17 (OL-7 rows 8 + 28, VERIFY.md on main 4a577dac, shots 014/015): the
  // guide drew an 880px column at x 200-1080 while desk, queue, bookings, backdated, transaction, staff, report and
  // documents all use the shell's 960px column at x 160 (1280 wide). `.staff-guide` now adds no width, margin or padding
  // of its own; it fills the shell column. Was: max-width 880px, margin 0 auto, padding 16px.
  it('SG-3 the six parts are shared cards in a .staff-guide grid that fills the shell column', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('{"version":"1.0.0","build":"0123456789","builtAt":"x","channel":"staging"}')),
    );
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => StaffGuide({ print: () => {} }), host);
    expect(host.querySelectorAll('.staff-guide > section.card').length).toBe(6);
    expect(await readHere('StaffGuide.tsx')).toMatch(/import '\.\.\/\.\.\/shared\/ui\/card\.css';/);
    const d = decls(await readHere('guide.css'), '.staff-guide');
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('gap')).toBe('16px');
    expect(d?.get('max-width')).toBe('none');
    expect(d?.get('margin')).toBe('0');
    expect(d?.get('padding')).toBe('0');
  });

  it('SG-4 headings and paragraphs inside a part have tidy margins', async () => {
    const css = await readHere('guide.css');
    expect(decls(css, '.staff-guide h2')?.get('margin')).toBe('0 0 8px');
    expect(decls(css, '.staff-guide p')?.get('margin')).toBe('0');
    expect(decls(css, '.staff-guide p + p')?.get('margin-top')).toBe('8px');
  });

  it('SG-5 paper has no card shadow', async () => {
    const stripped = (await readHere('guide.css')).replace(/\/\*[\s\S]*?\*\//g, '');
    expect(stripped).toMatch(/@media print\s*\{[\s\S]*\.staff-guide \.card\s*\{\s*box-shadow:\s*none;/);
  });
});
