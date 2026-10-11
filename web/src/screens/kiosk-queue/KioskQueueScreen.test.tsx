// V1-kiosk-queue-panel (#486): the page /kiosk-queue.html, design docs/design/xchange-v3-web.md
// §12.18.5 (v2: src/devices-public/kiosk-queue.html:1 of xchange-v2-source@4d0bdbb).
// Written before the implementation by the lane controller. Do not edit, delete or weaken; add
// tests in a file of your own.
//
// The page is booted the way the browser boots it: `web/pages/kiosk-queue.html` names
// `main.tsx`, which is imported into a document with `<div id="app">` and a stubbed `fetch` that
// answers like the server (`me`, `kiosk_notifications`, `kiosk_queue`). The files are reached
// through `import.meta.glob`, so a missing file fails an assertion here, not the whole file.
//
// Amended for SB-3 (#842) by the test-writer claude/5d45b017, on the FE lane controller
// claude/5edf7bb5's ruling (#842 issuecomment-6105121433, item 2): the pin "without ?branch= the
// branch is MAIN, and setQueueBranch(MAIN) wins over me().branch" becomes "without ?branch= the
// reads name me().branch (B-HOME)". Session branch design §14.4: no page names 'MAIN' itself.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `me` answer fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` →
// `employeeOf(…)` (`web/src/test-support/me.ts`); untyped, so tsc did not list it, but the queue card, the
// account bar and the queue store read `permissions`/`roles`. Nothing else changes.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { employeeOf, meOf } from '../../test-support/me';

const PAGES = import.meta.glob('../../../pages/kiosk-queue.html', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;
const TABLES = import.meta.glob('./i18n.ts', { eager: true }) as Record<
  string,
  { th?: Record<string, string>; en?: Record<string, string> }
>;
const MAIN = import.meta.glob('./main.tsx');
const STORE = import.meta.glob('../../shared/queue/queue-store.ts');

const disposers: Array<() => void> = [];

vi.mock('solid-js/web', async (importOriginal) => {
  const mod = await importOriginal<typeof import('solid-js/web')>();
  return {
    ...mod,
    render: (code: () => any, element: Node, init?: any, options?: any) => {
      const dispose = mod.render(code, element, init, options);
      disposers.push(dispose);
      return dispose;
    },
  };
});

const THAI = /[฀-๿]/;

/** v2's four strings of the page (kiosk-queue.html:1), verbatim. */
const V2 = {
  title: 'XC · คิวลูกค้าและเวลาทำรายการ',
  back: '← แอดมินหน้าร้าน',
  heading: 'คิวลูกค้าและเวลาทำรายการ',
  intro: 'รับเรื่องด้วยบัญชีของตนเอง เวลาเริ่มนับเมื่อรับเรื่อง และสิ้นสุดเมื่อบันทึกรายการสำเร็จ',
};

const ME = meOf({
  id: 'E1',
  role: 'FRONT',
  branch: 'B-HOME',
  name: 'Somchai',
});

function table(): { th: Record<string, string>; en: Record<string, string> } {
  const mod = TABLES['./i18n.ts'];
  expect(mod, 'web/src/screens/kiosk-queue/i18n.ts exists').toBeDefined();
  expect(mod!.th, 'i18n.ts exports th').toBeTypeOf('object');
  expect(mod!.en, 'i18n.ts exports en').toBeTypeOf('object');
  return { th: mod!.th!, en: mod!.en! };
}

/** The en text of the page's own key whose th text is `thai`. */
function english(thai: string): string {
  const { th, en } = table();
  const key = Object.keys(th).find((k) => th[k] === thai);
  expect(key, `a kiosk-queue key holds "${thai}"`).toBeDefined();
  return en[key!]!;
}

let urls: URL[] = [];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function serve(input: unknown): Response {
  const url = new URL(String(input), 'http://xc.test');
  urls.push(url);
  const branch = url.searchParams.get('branch') ?? '';
  switch (url.pathname) {
    case '/api/v3/views/me':
      return json({ version: 1, changed: true, data: ME });
    case '/api/v3/views/kiosk_notifications':
      return json({
        version: 7,
        changed: true,
        data: {
          serverNow: new Date().toISOString(),
          branchId: branch,
          branchName: branch,
          count: 1,
          latest: `${branch}-kq-1`,
          workingCount: 0,
          working: [],
          employee: employeeOf(ME),
        },
      });
    case '/api/v3/views/kiosk_queue':
      return json({
        version: 7,
        changed: true,
        data: [
          {
            id: `${branch}-kq-1`,
            queue: `${branch}-Q1`,
            orderNo: 'ORD-1',
            transactionNo: null,
            status: 'WAITING',
            type: 'BUY',
            currency: 'USD',
            foreign: '100.00',
            thb: '3325.00',
            staff: '',
            startedAt: null,
            completedAt: null,
            durationSeconds: null,
            canAccept: true,
            canContinue: false,
          },
        ],
      });
    case '/health/ready':
      // Injected ready answer: AppShell polls /health/ready on mount (#567)
      return json({
        status: 'ok',
        database: true,
        version: '1.0.0',
        build: '0123456789abcdef0123456789abcdef01234567',
        builtAt: '2026-10-08T01:02:03Z',
        channel: 'staging',
        ui: 'built',
      });
    default:
      return json({ error: { code: 'notFound', message: 'Not found' } }, 404);
  }
}

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

/** Boots the page at /kiosk-queue.html<search> in `lang`, as the browser would. */
async function boot(search: string, lang: 'th' | 'en' = 'th') {
  window.history.replaceState(null, '', `/kiosk-queue.html${search}`);
  localStorage.setItem('xc.lang', lang);
  const app = document.createElement('div');
  app.id = 'app';
  document.body.append(app);
  expect(MAIN['./main.tsx'], 'web/src/screens/kiosk-queue/main.tsx exists').toBeDefined();
  vi.resetModules();
  await MAIN['./main.tsx']!();
  await settle();
  expect(STORE['../../shared/queue/queue-store.ts'], 'queue-store.ts exists').toBeDefined();
  const store = (await STORE['../../shared/queue/queue-store.ts']!()) as { queueBranch: () => string };
  return { app, queueBranch: store.queueBranch };
}

function text(node: Element | null | undefined): string {
  return (node?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function queueReads(): URL[] {
  return urls.filter((u) => u.pathname === '/api/v3/views/kiosk_queue');
}

beforeAll(async () => {
  vi.stubGlobal('fetch', vi.fn(async (input: unknown) => serve(input)));
  await MAIN['./main.tsx']!();
  while (disposers.length) disposers.pop()?.();
  vi.unstubAllGlobals();
  document.body.innerHTML = '';
});

beforeEach(() => {
  vi.useFakeTimers({ now: Date.parse('2026-10-08T03:00:00.000Z') });
  urls = [];
  vi.stubGlobal('fetch', vi.fn(async (input: unknown) => serve(input)));
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.innerHTML = '';
  localStorage.removeItem('xc.lang');
  window.history.replaceState(null, '', '/');
});

describe('the page file', () => {
  it('declares /kiosk-queue.html, gate signed-in, and loads screens/kiosk-queue/main.tsx', () => {
    const raw = PAGES['../../../pages/kiosk-queue.html'];
    expect(raw, 'web/pages/kiosk-queue.html exists').toBeTypeOf('string');
    const doc = new DOMParser().parseFromString(raw!, 'text/html');
    expect(doc.querySelector('meta[name="xc-path"]')?.getAttribute('content')).toBe('/kiosk-queue.html');
    expect(doc.querySelector('meta[name="xc-gate"]')?.getAttribute('content')).toBe('signed-in');
    const scripts = [...doc.querySelectorAll('script')].map((s) => [s.getAttribute('type'), s.getAttribute('src')]);
    expect(scripts).toContainEqual(['module', '/src/screens/kiosk-queue/main.tsx']);
  });
});

describe("the page's own table kiosk-queue.*", () => {
  it('th holds v2’s four strings verbatim; en has the same keys, each in English', () => {
    const { th, en } = table();
    for (const thai of Object.values(V2)) expect(Object.values(th), thai).toContain(thai);
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [key, value] of Object.entries(en)) {
      expect(value, key).toBeTruthy();
      expect(value, key).not.toMatch(THAI);
    }
  });
});

describe('the booted page', () => {
  it("shows AppShell titled XC · …, the back link to /, the heading, the intro, and the panel's first page", async () => {
    const { app } = await boot('?branch=B9');
    expect(text(app.querySelector('h1'))).toBe(V2.title);
    const back = [...app.querySelectorAll('a')].find((a) => text(a) === V2.back);
    expect(back?.getAttribute('href')).toBe('/');
    const headings = [...app.querySelectorAll('h1,h2,h3,h4,h5,h6')].map((h) => text(h));
    expect(headings).toContain(V2.heading);
    expect(text(app)).toContain(V2.intro);
    expect(text(app)).toContain('รอรับเรื่อง');
    expect(headings).toContain('B9-Q1');
  });

  it('?branch= names the branch: the panel reads it, and the queue branch is set to it', async () => {
    const { queueBranch } = await boot('?branch=B9');
    const reads = queueReads();
    expect(reads.length).toBeGreaterThan(0);
    for (const u of reads) expect(u.searchParams.get('branch')).toBe('B9');
    expect(reads[0]!.searchParams.get('state')).toBe('waiting');
    expect(reads[0]!.searchParams.get('limit')).toBe('51');
    expect(queueBranch()).toBe('B9');
  });

  it('without ?branch= the reads name me().branch (B-HOME), and queueBranch() is B-HOME', async () => {
    const { app, queueBranch } = await boot('');
    const reads = queueReads();
    expect(reads.length).toBeGreaterThan(0);
    for (const u of reads) expect(u.searchParams.get('branch')).toBe('B-HOME');
    expect(queueBranch()).toBe('B-HOME');
    const headings = [...app.querySelectorAll('h1,h2,h3,h4,h5,h6')].map((h) => text(h));
    expect(headings).toContain('B-HOME-Q1');
  });

  it("in en, the page's strings come from its en table and its content has no Thai", async () => {
    const { app } = await boot('?branch=B9', 'en');
    expect(text(app.querySelector('h1'))).toBe(english(V2.title));
    const back = [...app.querySelectorAll('a')].find((a) => text(a) === english(V2.back));
    expect(back?.getAttribute('href')).toBe('/');
    const headings = [...app.querySelectorAll('h1,h2,h3,h4,h5,h6')].map((h) => text(h));
    expect(headings).toContain(english(V2.heading));
    expect(text(app)).toContain(english(V2.intro));
    const content = app.querySelector('main') ?? app.querySelector('.xc-content');
    expect(content).not.toBeNull();
    expect(text(content)).not.toMatch(THAI);
  });
});
