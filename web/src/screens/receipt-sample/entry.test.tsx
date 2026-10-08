// V1-receipt-sample (#398): the HTML entry (ticket §7, R1, S2, S4).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';

const entries = import.meta.glob<string>('../../../pages/receipt-sample.html', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function parse(): Document {
  const raw = entries['../../../pages/receipt-sample.html'];
  expect(raw, 'web/pages/receipt-sample.html exists').toBeTypeOf('string');
  return new DOMParser().parseFromString(raw!, 'text/html');
}

function metas(doc: Document, name: string): string[] {
  return Array.from(doc.querySelectorAll(`meta[name="${name}"]`)).map((m) => m.getAttribute('content') ?? '');
}

describe('V1-receipt-sample: web/pages/receipt-sample.html', () => {
  it('is English, titled XCHANGE • Exchange Ticket, and declares xc-path /receipt-sample with gate branch (R1, S2)', () => {
    const doc = parse();
    expect(doc.documentElement.getAttribute('lang')).toBe('en');
    expect(doc.title).toBe('XCHANGE • Exchange Ticket');
    expect(metas(doc, 'xc-path')).toEqual(['/receipt-sample']);
    expect(metas(doc, 'xc-gate')).toEqual(['branch']);
  });

  it('mounts the screen through /src/screens/receipt-sample/main.tsx, with no XC_BRANCH and no /_xc/ script (S4)', () => {
    const doc = parse();
    const scripts = Array.from(doc.querySelectorAll('script'));
    expect(
      scripts.map((s) => ({ src: s.getAttribute('src'), type: s.getAttribute('type') })),
    ).toContainEqual({ src: '/src/screens/receipt-sample/main.tsx', type: 'module' });
    for (const s of scripts) {
      expect(s.getAttribute('src') ?? '', 'no /_xc/ script').not.toMatch(/^\/_xc\//);
      expect(s.textContent ?? '', 'no window.XC_BRANCH').not.toContain('XC_BRANCH');
    }
  });
});
