// OL-8 bookings-375 (verification round 12, seen outside the 138 rows; audit row desk/staff 8 for the width): at 375 px
// the booking-row buttons wrap one Thai word per line ("ยกเลิก / การ / จอง", 58-76 px tall) because
// `.xc-booking-actions` is a single nowrap flex row of four buttons, and `.xc-slot-group` overflows (scrollWidth 324 in
// 293). At 1280 px the page wrapper adds its own 24 px gutter (toolbar 944 px at x=168), breaking the one content width
// the shell owns. Measured in Chromium on feat/one-look-8 13598bc5 with this screen's CSS. Written before the fix by
// claude/d5c44a22 for the sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

async function readHere(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
}

/** The declarations of every rule whose selector list contains `selector`, merged in source order. */
function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  let out: Map<string, string> | undefined;
  while ((m = re.exec(css)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    out ??= new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
  }
  return out;
}

describe('desk-bookings at phone width', () => {
  it('BP-1 .xc-booking-actions wraps its buttons onto more lines instead of squeezing them', async () => {
    const d = decls(await readHere('desk-bookings.css'), '.xc-booking-actions');
    expect(d?.get('display')).toBe('flex');
    expect(d?.get('flex-wrap')).toBe('wrap');
    expect(d?.get('min-width')).toBe('0');
  });

  it('BP-2 every booking-row button keeps its label on one line', async () => {
    const d = decls(await readHere('desk-bookings.css'), '.xc-booking-actions > button');
    expect(d?.get('white-space')).toBe('nowrap');
  });

  it('BP-3 .xc-desk-bookings has no side gutter or width of its own: the shell owns them', async () => {
    const d = decls(await readHere('desk-bookings.css'), '.xc-desk-bookings');
    expect(d?.get('padding')).toBe('var(--xc-space-4) 0');
    expect(d?.get('max-width')).toBe('none');
    expect(d?.get('margin')).toBe('0');
  });

  it('BP-4 guard: the slot group still clips its rounded corners and the row still wraps info above actions', async () => {
    const css = await readHere('desk-bookings.css');
    expect(decls(css, '.xc-slot-group')?.get('overflow')).toBe('hidden');
    expect(decls(css, '.xc-booking-item')?.get('flex-wrap')).toBe('wrap');
  });
});
