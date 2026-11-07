// OL-7 row 65 (verification of 2026-11-07; re-measured on feat/one-look-7 52680054 with xc-v3 data, read-only):
// customer ids are CX-0000000N codes (no raw UUIDs with real data), and since admin-a-css (499aaac2) a wide table
// shows that it scrolls. The customers table is still 840px wide, though: it fits at 1280, but at 1100 it sits in a
// 762px box and the "ดูประวัติ" buttons are cut at the card edge, so the row's main action needs a sideways scroll at
// a common laptop width. The headers now wrap (they are the widest cells: "ยอดแลกเงินรวม (THB)", "รายการสำเร็จ")
// and the cells take 14px side padding in place of the admin's 20px. Rendered with this: 1100 and 1024 fit; a phone
// still scrolls inside `.table-scroll`.
// Written before the fix by claude/bbe9ce8b for the OL sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

function decls(selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = css.replace(/\/\*[\s\S]*?\*\//g, '');
  let out: Map<string, string> | undefined;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim().replace(/\s+/g, ' ')).includes(selector)) continue;
    out ??= new Map();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
  }
  return out;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'customers.css', 'utf8');
});

describe('customers table fits a laptop card', () => {
  it('CU-1 the headers wrap, with 12px 14px padding (outranks admin.css .admin-main th)', () => {
    const d = decls('.xc-admin .xc-content .admin-main .customers-table th');
    expect(d, 'rule `.xc-admin .xc-content .admin-main .customers-table th`').toBeDefined();
    expect(d!.get('white-space')).toBe('normal');
    expect(d!.get('padding')).toBe('12px 14px');
  });

  it('CU-2 the cells take 14px padding (outranks admin.css .admin-main td)', () => {
    const d = decls('.xc-admin .xc-content .admin-main .customers-table td');
    expect(d, 'rule `.xc-admin .xc-content .admin-main .customers-table td`').toBeDefined();
    expect(d!.get('padding')).toBe('14px');
  });

  it('CU-3 the table still sits in .table-scroll for phones (guard)', async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    const src = fs.readFileSync(here + 'Customers.tsx', 'utf8');
    expect(src).toMatch(/<div class="table-scroll">\s*<table class="customers-table">/);
  });
});
