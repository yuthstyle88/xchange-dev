// V1-devices-screen (#404): the HTML entry of /devices (ticket §7 `web/pages/devices.html`, D1, V11, V12).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
import { describe, expect, it } from 'vitest';

const entries = import.meta.glob<string>('../../../pages/devices.html', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function parse(): Document {
  const raw = entries['../../../pages/devices.html'];
  expect(raw, 'web/pages/devices.html exists').toBeTypeOf('string');
  return new DOMParser().parseFromString(raw!, 'text/html');
}

function metas(doc: Document, name: string): string[] {
  return Array.from(doc.querySelectorAll(`meta[name="${name}"]`)).map((m) => m.getAttribute('content') ?? '');
}

describe('V1-devices-screen: web/pages/devices.html (#404 §7)', () => {
  it("is Thai, titled as v2's page, and declares xc-path /devices behind the signed-in gate (D1)", () => {
    const doc = parse();
    expect(doc.documentElement.getAttribute('lang')).toBe('th');
    expect(doc.title).toBe('XCHANGE · งานอุปกรณ์');
    expect(metas(doc, 'xc-path')).toEqual(['/devices']);
    expect(metas(doc, 'xc-gate')).toEqual(['signed-in']);
  });

  it('mounts the screen through /src/screens/devices/main.tsx and carries none of the v2 add-on scripts (V11, V12)', () => {
    const doc = parse();
    const scripts = Array.from(doc.querySelectorAll('script[src]')).map((s) => ({
      src: s.getAttribute('src'),
      type: s.getAttribute('type'),
    }));
    expect(scripts).toContainEqual({ src: '/src/screens/devices/main.tsx', type: 'module' });
    for (const s of scripts) {
      expect(s.src, 'no /_xc/ script').not.toMatch(/^\/_xc\//);
      expect(s.src, 'no /account-bar.js').not.toBe('/account-bar.js');
      expect(s.src, 'no v2 /devices.js').not.toBe('/devices.js');
    }
  });
});
