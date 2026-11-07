// OL-7 rows 131 + 132 (verification ~/xc-verify-138 rows 131/132, shots 080 / 081): /c (copy link) and /q (request
// QR) render a lone card with no app header; every other customer page has the customer shell (header with logo and
// language switch, sign-in / notifications bar, bottom navigation). Root: copy-link/main.tsx and request-qr/main.tsx
// render the screen without <CustomerShell>, and CustomerRoute has no '/c' or '/q'. The screens keep their 100vh
// centring, which inside the shell would push the bottom navigation below the fold.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let fs: NodeFs;
let here = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
});

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  document.body.innerHTML = '<div id="root"></div>';
  vi.spyOn(globalThis, 'fetch').mockImplementation(async () =>
    new Response(JSON.stringify({ error: { code: 'unauthorized', message: 'unauthorized' } }), {
      status: 401,
      headers: { 'content-type': 'application/json' },
    }),
  );
});

afterEach(() => {
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

const WAIT = { timeout: 1000, interval: 20 };

/** The root rule of a screen stylesheet: the declarations of the first block whose selector is exactly `sel`. */
function rootRule(file: string, sel: string): string {
  const css = fs.readFileSync(here + file, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  const m = new RegExp(`(?:^|\\})\\s*${sel.replace('.', '\\.')}\\s*\\{([^}]*)\\}`).exec(css);
  return m?.[1] ?? '';
}

describe('CQ /c and /q inside the customer shell (rows 131, 132)', () => {
  it('CQ-1 /c renders the customer shell: app header with the logo, bottom navigation, the card inside <main>', async () => {
    await import('../copy-link/main');
    await vi.waitFor(() => expect(document.querySelector('.xc-customer header.xc-shell-header')).not.toBeNull(), WAIT);
    expect(document.querySelector('.xc-customer-logo img')).not.toBeNull();
    expect(document.querySelector('nav.xc-customer-nav')).not.toBeNull();
    expect(document.querySelector('main.xc-customer-main .xc-copy-link-card')).not.toBeNull();
    expect(document.querySelector('.xc-customer-nav [aria-current="page"]'), 'no tab is marked current').toBeNull();
  });

  it('CQ-2 /q renders the customer shell the same way', async () => {
    await import('../request-qr/main');
    await vi.waitFor(() => expect(document.querySelector('.xc-customer header.xc-shell-header')).not.toBeNull(), WAIT);
    expect(document.querySelector('.xc-customer-logo img')).not.toBeNull();
    expect(document.querySelector('nav.xc-customer-nav')).not.toBeNull();
    expect(document.querySelector('main.xc-customer-main .xc-request-qr-card')).not.toBeNull();
    expect(document.querySelector('.xc-customer-nav [aria-current="page"]'), 'no tab is marked current').toBeNull();
  });

  it('CQ-3 inside the shell the screens no longer fill the viewport: card at the top, no 100vh', () => {
    for (const [file, sel] of [
      ['../copy-link/copy-link.css', '.xc-copy-link'],
      ['../request-qr/request-qr.css', '.xc-request-qr'],
    ] as const) {
      const body = rootRule(file, sel);
      expect(body, `${sel} root rule found`).not.toBe('');
      expect(body, `${sel} min-height`).not.toMatch(/min-height\s*:\s*100vh/);
      expect(body, `${sel} align-items`).toMatch(/align-items\s*:\s*flex-start/);
    }
  });
});
