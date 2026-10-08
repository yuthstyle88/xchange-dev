// V1-staff-guide (#397): the two HTML entries, one per path, both mounting the same screen (ticket §3, §7, R2, R4).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';

const entries = import.meta.glob<string>(['../../../pages/staff-guide.html', '../../../pages/staff-guide-html.html'], {
  query: '?raw',
  import: 'default',
  eager: true,
});

function parse(file: string): Document {
  const raw = entries[`../../../pages/${file}`];
  expect(raw, `web/pages/${file} exists`).toBeTypeOf('string');
  return new DOMParser().parseFromString(raw!, 'text/html');
}

function metas(doc: Document, name: string): string[] {
  return Array.from(doc.querySelectorAll(`meta[name="${name}"]`)).map((m) => m.getAttribute('content') ?? '');
}

describe.each([
  ['staff-guide.html', '/staff-guide'],
  ['staff-guide-html.html', '/staff-guide.html'],
])('V1-staff-guide: web/pages/%s', (file, path) => {
  it(`is Thai, titled คู่มือพนักงาน XC Desk v2, and declares xc-path ${path} with gate signed-in (G1, R2)`, () => {
    const doc = parse(file);
    expect(doc.documentElement.getAttribute('lang')).toBe('th');
    expect(doc.title).toBe('คู่มือพนักงาน XC Desk v2');
    expect(metas(doc, 'xc-path')).toEqual([path]);
    expect(metas(doc, 'xc-gate')).toEqual(['signed-in']);
  });

  it('mounts the screen through /src/screens/staff-guide/main.tsx and carries no v2 add-on script (R4, R5)', () => {
    const doc = parse(file);
    const scripts = Array.from(doc.querySelectorAll('script[src]')).map((s) => ({
      src: s.getAttribute('src'),
      type: s.getAttribute('type'),
    }));
    expect(scripts).toContainEqual({ src: '/src/screens/staff-guide/main.tsx', type: 'module' });
    for (const s of scripts) {
      expect(s.src, 'no /_xc/ script').not.toMatch(/^\/_xc\//);
      expect(s.src, 'no /account-bar.js').not.toBe('/account-bar.js');
    }
  });
});
