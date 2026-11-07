// OL-6 K1 (audit #564 issuecomment-6702970101, shot 057; owner ruling issuecomment-6703173879 item 4: kiosk touch
// screen fixed in this lane): api/static/kiosk/kiosk.css, the stylesheet the server sends for /kiosk.css
// (api/tests/pages_kiosk_test.rs:56-58 serves it byte for byte). Fixes at 430 px:
//   - the build stamp (#xc-version, a fixed overlay from pages.rs add_version_line) sits on the keypad "9" key;
//   - the language search field is half width in the 2-column .kiosk-lang grid, so its placeholder is cut;
//   - selects show the native chevron next to styled tiles;
//   - a disabled button is opacity .5 (white on light grey, near illegible).
// The fix is a K1 block at the END of kiosk.css. pages.rs and kiosk-xchange.css are not touched.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let block = '';
let whole = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  whole = fs.readFileSync(here + '../../../api/static/kiosk/kiosk.css', 'utf8');
  const at = whole.indexOf('/* OL-6 K1');
  block = at >= 0 ? whole.slice(at).replace(/\/\*[\s\S]*?\*\//g, '') : '';
});

const rule = (css: string, selector: string) => {
  const esc = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(^|[}\\s])${esc}\\s*\\{([^}]*)\\}`).exec(css)?.[2] ?? '';
};

describe('K1 kiosk touch screen fixes (api/static/kiosk/kiosk.css)', () => {
  it('K1-0 the fixes are one block, starting "/* OL-6 K1", at the end of kiosk.css', () => {
    expect(block, 'K1 block exists').not.toBe('');
    expect(whole.trimEnd().endsWith('}'), 'nothing after the block but rules').toBe(true);
  });

  it('K1-1 the build stamp leaves the keypad: #xc-version is in the page flow, centred', () => {
    const r = rule(block, '#xc-version');
    expect(r).toMatch(/position:\s*static\s*!important/);
    expect(r).toMatch(/text-align:\s*center/);
  });

  it('K1-2 at ≤760px the language search field spans both grid columns', () => {
    const media = /@media\s*\(max-width:\s*760px\)\s*\{([\s\S]*)\}\s*$/.exec(block.trim())?.[1] ?? '';
    expect(rule(media, '.kiosk-lang input')).toMatch(/grid-column:\s*1\s*\/\s*-1/);
  });

  it('K1-3 selects drop the native chevron and draw their own', () => {
    const r = rule(block, 'select');
    expect(r).toMatch(/appearance:\s*none/);
    expect(r).toMatch(/background-image:\s*url\("data:image\/svg\+xml,/);
    expect(r).toMatch(/padding-right:\s*48px/);
  });

  it('K1-4 a disabled button keeps readable colours, no opacity', () => {
    const r = rule(block, 'button:disabled');
    expect(r).toMatch(/opacity:\s*1/);
    expect(r).toMatch(/background:\s*var\(--xc-gray-100\)/);
    expect(r).toMatch(/color:\s*var\(--xc-gray-600\)/);
  });
});
