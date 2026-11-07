// OL-6 L7 (design docs/design/xchange-one-look-layout.md §3 L7): each desk step's form and the summary are cards,
// as the owner-approved mock-up (docs/design/one-look-mock/index.html:170-196, `.card`). Owner 2026-11-07 ~12:20 ICT on
// step 04 payment: "มันควรทำเป็นการ์ดนะ"; on step 02: "เหมือน กัน". The card look comes from the shared part
// `web/src/shared/ui/card.css` (`.xc-theme .card`), never a copy in desk.css.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../shared/i18n';
import { warmModules } from '../../shared/testing/warm-modules';
import { withOpenShift } from './shift/open-shift.testing';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

const ME = { kind: 'user', id: 'u-1', role: '', branch: 'MAIN', name: 'Ploy', username: 'ploy', branchName: 'XC-B', branches: [] };
const CONFIG = {
  branchId: 'MAIN',
  branchName: 'XC-B',
  shiftOpen: true,
  adminUrl: '/?branch=MAIN',
  rates: [{ code: 'USD', unitSize: '1', buy: '32.10000000', sell: '32.90000000', publication: 'pub-1' }],
  paymentMethods: [],
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function fakeServer() {
  const fn = vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input).split('?')[0];
    if (path === '/api/v3/views/me') return json(200, { version: 1, changed: true, data: ME });
    if (path === '/api/v3/views/desk_config' || path === '/api/v3/views/staff_desk_config') {
      return json(200, { version: 1, changed: true, data: CONFIG });
    }
    return json(404, { error: { code: 'notFound', message: 'nf' } });
  });
  vi.stubGlobal('fetch', withOpenShift(fn));
}

async function readHere(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8');
}

let app: HTMLDivElement;

beforeAll(() => warmModules(() => import('./main')), 60_000);

beforeEach(() => {
  setLang('th');
  vi.resetModules();
  app = document.createElement('div');
  app.id = 'app';
  document.body.appendChild(app);
});

afterEach(() => {
  app.remove();
  window.history.replaceState({}, '', '/');
});

describe('L7 desk step form and summary are cards', () => {
  it('L7-1 the step panel is a shared card: section.panel carries class "card"', async () => {
    window.history.replaceState({}, '', '/desk');
    fakeServer();
    await import('./main');
    await vi.waitFor(() => expect(app.querySelector('section.panel')).not.toBeNull());
    expect(app.querySelector('section.panel')?.classList.contains('card')).toBe(true);
  });

  it('L7-2 the summary is a shared card: aside.desk-aside carries class "card"', async () => {
    window.history.replaceState({}, '', '/desk');
    fakeServer();
    await import('./main');
    await vi.waitFor(() => expect(app.querySelector('aside.desk-aside')).not.toBeNull());
    expect(app.querySelector('aside.desk-aside')?.classList.contains('card')).toBe(true);
  });

  it('L7-3 DeskPage.tsx imports the shared card.css (the look is not copied into desk.css)', async () => {
    const src = await readHere('DeskPage.tsx');
    expect(src).toMatch(/import ['"]\.\.\/\.\.\/shared\/ui\/card\.css['"];/);
    const css = (await readHere('desk.css')).replace(/\/\*[\s\S]*?\*\//g, '');
    expect(css, 'desk.css declares no box-shadow (the card look lives in card.css)').not.toMatch(/box-shadow\s*:/);
  });

  it('L7-4 at 800 px or less the desk cards pad 16px, as the mock-up\'s phone .card', async () => {
    const css = (await readHere('desk.css')).replace(/\/\*[\s\S]*?\*\//g, '');
    const block = /@media\s*\(max-width:\s*800px\)\s*\{([\s\S]*?\})\s*\}/.exec(css)?.[1] ?? '';
    const rule = /\.desk-body\s+\.card\s*\{([^}]*)\}/.exec(block)?.[1] ?? '';
    expect(rule, '.desk-body .card rule inside @media (max-width: 800px)').not.toBe('');
    expect(rule).toMatch(/padding\s*:\s*16px\s*(;|$)/);
  });
});
