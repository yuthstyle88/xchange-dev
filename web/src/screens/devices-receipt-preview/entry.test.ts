// OL-6 wave B devices-receipt-page (audit admin B row 054): the HTML entry of the slip preview page. The server serves
// every `web/pages/*.html` by its `xc-path` and `xc-gate` (api/build_support/screens.rs:48-74 builds the table,
// api/src/presentation/screens.rs:122 serves it), so this file is the whole route. Written before the fix by
// claude/94389a6b. Do not edit, delete or weaken; add your own elsewhere.
import { describe, expect, it } from 'vitest';

const entries = import.meta.glob<string>('../../../pages/devices-receipt-preview.html', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function parse(): Document {
  const raw = entries['../../../pages/devices-receipt-preview.html'];
  expect(raw, 'web/pages/devices-receipt-preview.html exists').toBeTypeOf('string');
  return new DOMParser().parseFromString(raw!, 'text/html');
}

function metas(doc: Document, name: string): string[] {
  return Array.from(doc.querySelectorAll(`meta[name="${name}"]`)).map((m) => m.getAttribute('content') ?? '');
}

describe('devices-receipt-preview entry', () => {
  it('RP-E1 is Thai, titled, and declares xc-path /devices/receipt-preview behind the same gate as /devices', () => {
    const doc = parse();
    expect(doc.documentElement.getAttribute('lang')).toBe('th');
    expect(doc.title).toBe('XCHANGE · ตัวอย่างสลิป 80 มม.');
    expect(metas(doc, 'xc-path')).toEqual(['/devices/receipt-preview']);
    expect(metas(doc, 'xc-gate')).toEqual(['signed-in']);
  });

  it('RP-E2 mounts through /src/screens/devices-receipt-preview/main.tsx and carries no v2 script', () => {
    const doc = parse();
    const scripts = Array.from(doc.querySelectorAll('script[src]')).map((s) => ({
      src: s.getAttribute('src'),
      type: s.getAttribute('type'),
    }));
    expect(scripts).toEqual([{ src: '/src/screens/devices-receipt-preview/main.tsx', type: 'module' }]);
  });
});
