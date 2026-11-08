// OL-8 content-width (audit row desk/staff 8, verification round 12: h1 at x=144 in a 992 px column; desk, queue,
// backdated, staff and report cards at x=160 and 960 wide; guide cards 992 at x=144; documents card 700). One rule:
// the shell owns the content width and the side gutter. `.xc-shell-main` is 1008 px wide with 24 px side padding, so
// the content column is 960 px (x 160-1120 at 1280 px) and the 160 px right gutter holds the folded queue pill
// (queue-card.css `.queue-card.minimized`, max 140 px at right 20 px). Screen wrappers set no side padding and no
// max-width of their own. Written before the fix by claude/d5c44a22 for the sub-controller claude/1cc51615.
// Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

/** Reads a file relative to this test's folder (web/src/shared/ui/). */
async function readRel(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
}

/** The declarations of the first rule whose selector list contains `selector`, in `css`. */
function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(css)) !== null) {
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

/** Splits `css` into the rules outside any `@media` block and the bodies of its `@media` blocks. */
function split(css: string): { outside: string; media: string } {
  let outside = '';
  let media = '';
  let from = 0;
  let at = css.indexOf('@media');
  while (at >= 0) {
    outside += css.slice(from, at);
    const open = css.indexOf('{', at);
    let depth = 1;
    let i = open + 1;
    while (i < css.length && depth > 0) {
      if (css[i] === '{') depth += 1;
      if (css[i] === '}') depth -= 1;
      i += 1;
    }
    media += css.slice(open + 1, i - 1) + '\n';
    from = i;
    at = css.indexOf('@media', i);
  }
  return { outside: outside + css.slice(from), media };
}

const base = (css: string): string => split(css).outside;
const mediaBodies = (css: string): string => split(css).media;

/** The left and right of a `padding` shorthand (1 to 4 values). */
function sides(padding: string): { left: string; right: string } {
  const v = padding.trim().split(/\s+/);
  const right = v[1] ?? v[0]!;
  const left = v[3] ?? right;
  return { left, right };
}

const px = (v: string | undefined): number => Number(/^(-?\d+(?:\.\d+)?)px$/.exec(v ?? '')?.[1] ?? NaN);

/** Screen wrappers that sit directly in `main.xc-shell-main`, the file that styles them, and their selector. */
const WRAPPERS: ReadonlyArray<{ file: string; selector: string; padding: string }> = [
  { file: '../../screens/desk/desk.css', selector: '.desk-page', padding: '16px 0' },
  { file: '../../screens/desk/queue/queue.css', selector: '.desk-queue', padding: '16px 0' },
  { file: '../../screens/staff-report/staff-report.css', selector: '.staff-report', padding: '16px 0' },
  { file: '../../screens/staff/staff.css', selector: '.xc-staff-container', padding: '1.5rem 0' },
  { file: '../../screens/documents/documents.css', selector: '.documents-screen', padding: '16px 0' },
  { file: '../../screens/staff-guide/guide.css', selector: '.staff-guide', padding: '0' },
];

describe('content-width: one content column, owned by the shell', () => {
  it('CW-1 .xc-shell-main is 1008 px wide, centred, border-box, padding 24px 24px 48px', async () => {
    const d = decls(base(await readRel('shell.css')), '.xc-shell-main');
    expect(d?.get('max-width')).toBe('1008px');
    expect(d?.get('margin')).toBe('auto');
    expect(d?.get('box-sizing')).toBe('border-box');
    expect(d?.get('padding')).toBe('24px 24px 48px');
  });

  it('CW-2 at 1280 px the content column is 960 px at x 160-1120, leaving 160 px for the folded queue pill', async () => {
    const d = decls(base(await readRel('shell.css')), '.xc-shell-main')!;
    const max = px(d.get('max-width'));
    const { left, right } = sides(d.get('padding')!);
    const x0 = (1280 - max) / 2 + px(left);
    const x1 = 1280 - (1280 - max) / 2 - px(right);
    expect([x0, x1, x1 - x0]).toEqual([160, 1120, 960]);
    const pill = decls(await readRel('queue-card.css'), '.queue-card.minimized');
    const cardRight = px(decls(base(await readRel('queue-card.css')), '.queue-card')?.get('right'));
    expect(1280 - cardRight - px(pill?.get('max-width'))).toBeGreaterThanOrEqual(x1);
  });

  it.each(WRAPPERS)('CW-3 $selector ($file) has no side padding of its own: padding $padding', async (w) => {
    const d = decls(base(await readRel(w.file)), w.selector);
    expect(d, `${w.selector} rule exists`).toBeDefined();
    expect(d!.get('padding')).toBe(w.padding);
    const s = sides(d!.get('padding')!);
    expect([s.left, s.right]).toEqual(['0', '0']);
    for (const k of ['padding-left', 'padding-right', 'padding-inline']) expect(d!.has(k), k).toBe(false);
  });

  it.each(WRAPPERS)('CW-4 $selector ($file) sets no max-width other than none', async (w) => {
    const d = decls(base(await readRel(w.file)), w.selector)!;
    const mw = d.get('max-width');
    expect(mw === undefined || mw === 'none', `max-width: ${mw}`).toBe(true);
  });

  it.each(WRAPPERS)('CW-5 no @media block in $file gives $selector side padding or a max-width back', async (w) => {
    const d = decls(mediaBodies(await readRel(w.file)), w.selector);
    if (!d) return;
    for (const k of ['padding-left', 'padding-right', 'padding-inline']) expect(d.has(k), k).toBe(false);
    if (d.has('padding')) expect(Object.values(sides(d.get('padding')!))).toEqual(['0', '0']);
    const mw = d.get('max-width');
    expect(mw === undefined || mw === 'none', `max-width: ${mw}`).toBe(true);
  });
});
