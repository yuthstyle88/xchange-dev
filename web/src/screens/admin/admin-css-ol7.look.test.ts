// OL-7 admin-css (verification of 2026-11-07, rows 38, 45, 48 and 65; re-checked on feat/one-look-7 06b1e00b):
// - Row 38: the admin footer's "ตัวอย่างใบเสร็จ" is a bare muted link (`.admin-footer a`: muted, no decoration). It
//   opens a new tab like the header's "ข้อมูลและการเชื่อมต่อ ↗", so it now reads like that link: brand colour, 600, and
//   a trailing ↗.
// - Row 45: the cash history `<details class="workspace-history">` (and `.rate-history`) still shows the browser's ▶
//   marker. The summary drops the native marker and draws a chevron on its right that turns when open.
// - Rows 48 and 65: a `.table-scroll` that overflows scrolls, but macOS hides overlay scrollbars, so nothing says
//   more columns are there. Every admin `.table-scroll` now shows edge shadows while it can scroll (local/scroll
//   background layers; no script).
// Rules are read outside @media blocks, merged in source order as the cascade would. Written before the fix by
// claude/bbe9ce8b for the OL sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

/** admin.css without comments and without @media / @supports blocks. */
function topLevel(src: string): string {
  let s = src.replace(/\/\*[\s\S]*?\*\//g, '');
  for (;;) {
    const at = s.search(/@(media|supports)[^{]*\{/);
    if (at < 0) return s;
    let i = s.indexOf('{', at) + 1;
    let depth = 1;
    while (i < s.length && depth > 0) {
      if (s[i] === '{') depth += 1;
      if (s[i] === '}') depth -= 1;
      i += 1;
    }
    s = s.slice(0, at) + s.slice(i);
  }
}

/** Every top-level rule whose selector list holds `selector`, merged in source order. */
function decls(selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = topLevel(css);
  let out: Map<string, string> | undefined;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim().replace(/\s+/g, ' ')).includes(selector)) continue;
    out ??= new Map();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
  }
  return out;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'admin.css', 'utf8');
});

describe('row 38: the footer link reads like the header link', () => {
  it('AC-1 .admin-footer a: brand colour, 600, no underline', () => {
    const d = decls('.xc-admin .xc-content .admin-footer a');
    expect(d, 'rule `.xc-admin .xc-content .admin-footer a`').toBeDefined();
    expect(d!.get('color')).toBe('var(--xc-brand-text)');
    expect(d!.get('font-weight')).toBe('600');
    expect(d!.get('text-decoration')).toBe('none');
  });

  it('AC-2 .admin-footer a::after adds " ↗"', () => {
    const d = decls('.xc-admin .xc-content .admin-footer a::after');
    expect(d, 'rule `.xc-admin .xc-content .admin-footer a::after`').toBeDefined();
    expect(d!.get('content')).toBe('" ↗"');
  });
});

describe('row 45: the history summary has a drawn chevron, not the browser marker', () => {
  for (const box of ['workspace-history', 'rate-history']) {
    it(`AC-3 .${box} > summary: no list marker, a flex row with the text left and the chevron right`, () => {
      const d = decls(`.xc-admin .xc-content .${box} > summary`);
      expect(d, `rule \`.${box} > summary\``).toBeDefined();
      expect(d!.get('list-style')).toBe('none');
      expect(d!.get('display')).toBe('flex');
      expect(d!.get('align-items')).toBe('center');
      expect(d!.get('justify-content')).toBe('space-between');
      expect(d!.get('gap')).toBe('12px');
    });

    it(`AC-4 .${box} > summary::-webkit-details-marker is hidden`, () => {
      expect(decls(`.xc-admin .xc-content .${box} > summary::-webkit-details-marker`)?.get('display')).toBe('none');
    });

    it(`AC-5 .${box} > summary::after is an 8px chevron pointing down; [open] turns it up`, () => {
      const d = decls(`.xc-admin .xc-content .${box} > summary::after`);
      expect(d, `rule \`.${box} > summary::after\``).toBeDefined();
      expect(d!.get('content')).toBe('""');
      expect(d!.get('width')).toBe('8px');
      expect(d!.get('height')).toBe('8px');
      expect(d!.get('border-right')).toBe('2px solid currentColor');
      expect(d!.get('border-bottom')).toBe('2px solid currentColor');
      expect(d!.get('transform')).toBe('rotate(45deg)');
      expect(d!.get('flex')).toBe('none');
      expect(d!.get('margin-right'), 'the turned square stays inside the summary').toBe('2px');
      const open = decls(`.xc-admin .xc-content .${box}[open] > summary::after`);
      expect(open?.get('transform')).toBe('rotate(-135deg)');
    });
  }
});

describe('rows 48 and 65: a scrolling table shows that it scrolls', () => {
  it('AC-6 .table-scroll: two local cover layers and two scroll shadow layers on the surface colour', () => {
    const d = decls('.xc-admin .xc-content.xc-content .table-scroll');
    expect(d, 'rule `.xc-admin .xc-content.xc-content .table-scroll`').toBeDefined();
    expect(d!.get('overflow-x')).toBe('auto');
    expect(d!.get('background')).toBe(
      'linear-gradient(to right, var(--xc-surface) 30%, transparent) left center / 40px 100% no-repeat local, ' +
        'linear-gradient(to left, var(--xc-surface) 30%, transparent) right center / 40px 100% no-repeat local, ' +
        'radial-gradient(farthest-side at 0 50%, var(--xc-gray-300), transparent) left center / 12px 100% no-repeat scroll, ' +
        'radial-gradient(farthest-side at 100% 50%, var(--xc-gray-300), transparent) right center / 12px 100% no-repeat scroll, ' +
        'var(--xc-surface)',
    );
  });
});
