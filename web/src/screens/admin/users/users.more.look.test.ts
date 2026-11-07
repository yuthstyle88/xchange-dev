// OL-7 admin-users-more, the look half (see UsersScreen.more-menu.test.tsx for the behaviour). With reset, grant, suspend
// and disable in the "more" menu, a row's own buttons fit one line, so they no longer wrap (rows were 149px at 1280px with
// three lines of buttons). The table sits in `.users-table-scroll { overflow-x: auto }`, which clips in both directions,
// so a dropdown laid over the rows would be cut off on the last rows: the open menu stays in the flow under its toggle
// (a column of buttons) and makes only that row taller while it is open. Written before the implementation by
// claude/94389a6b for the sub-controller claude/1cc51615. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

/** The value of `prop` for `selector` in top-level rules (outside @media), the last one in the file winning. */
function value(selector: string, prop: string): string | undefined {
  const top = css.replace(/@media[^{]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, '');
  let v: string | undefined;
  for (const m of top.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0 && d.slice(0, i).trim().toLowerCase() === prop) v = d.slice(i + 1).trim();
    }
  }
  return v;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'users.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
});

describe('users row actions and the more menu', () => {
  it('UL-1 the row buttons stay on one line', () => {
    expect(value('.users-row-actions', 'flex-wrap')).toBe('nowrap');
  });

  it('UL-2 the open menu is a column of buttons with 4px between them, under its toggle', () => {
    expect(value('.users-more', 'display')).toBe('flex');
    expect(value('.users-more', 'flex-direction')).toBe('column');
    expect(value('.users-more', 'align-items')).toBe('flex-end');
    expect(value('.users-more-menu', 'display')).toBe('flex');
    expect(value('.users-more-menu', 'flex-direction')).toBe('column');
    expect(value('.users-more-menu', 'gap')).toBe('4px');
    expect(value('.users-more-menu', 'margin-top')).toBe('6px');
  });

  it('UL-3 the menu is not laid over the rows (the scroll box would clip it)', () => {
    expect(value('.users-more-menu', 'position') ?? 'static').toBe('static');
    expect(value('.users-more', 'position') ?? 'static').toBe('static');
  });
});
