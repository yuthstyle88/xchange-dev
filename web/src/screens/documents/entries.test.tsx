// #1349 V3-documents (design `docs/design/xchange-v3-wave3-report-documents.md` §A.3, §A.6, file **E** of §A.8): the
// two HTML entries, and each page mounted through its real entry script. Written before the implementation by the
// test-writer claude/5d45b017, on the FE lane controller's order (claude/5edf7bb5). Do not edit, delete or weaken.
//
// [required]:
//   - `web/pages/documents.html`: `xc-path` `/documents`, `xc-gate` `signed-in`, `<script type="module"
//     src="/src/screens/documents/owner.tsx">`.
//   - `web/pages/staff-documents.html`: `xc-path` `/staff/documents`, `xc-gate` `anyone` (D-A4: the view refuses,
//     the screen goes back to `/staff`), script `/src/screens/documents/staff.tsx`.
//   - Each entry renders into `#app`, inside `AppShell`. It imports and renders `AppShell` itself; that is LK-1's
//     direct form (`web/src/shared/ui/shell-look.test.ts`), so no shared gate file changes. It reads the page's query
//     with `URLSearchParams.get` (the **first** of a repeated name, #139), builds its own `ApiClient`, and passes the
//     browser's `location.replace` as `replace`.
//
// The real-entry tests stub the global `fetch` that the entry's `ApiClient` calls, set the page URL, import the entry
// module fresh, and find the list on the page.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../shared/i18n';

const entries = import.meta.glob<string>(['../../../pages/documents.html', '../../../pages/staff-documents.html'], {
  query: '?raw',
  import: 'default',
  eager: true,
});
const scripts = import.meta.glob<string>(['./owner.tsx', './staff.tsx'], {
  query: '?raw',
  import: 'default',
  eager: true,
});
const modules = import.meta.glob(['./owner.tsx', './staff.tsx']);

function parse(file: string): Document {
  const raw = entries[`../../../pages/${file}`];
  expect(raw, `web/pages/${file} exists`).toBeTypeOf('string');
  return new DOMParser().parseFromString(raw!, 'text/html');
}

function metas(doc: Document, name: string): string[] {
  return Array.from(doc.querySelectorAll(`meta[name="${name}"]`)).map((m) => m.getAttribute('content') ?? '');
}

describe.each([
  ['documents.html', '/documents', 'signed-in', '/src/screens/documents/owner.tsx'],
  ['staff-documents.html', '/staff/documents', 'anyone', '/src/screens/documents/staff.tsx'],
])('V3-documents E: web/pages/%s', (file, path, gate, script) => {
  it(`declares xc-path ${path}, gate ${gate}, and mounts ${script} into #app`, () => {
    const doc = parse(file);
    expect(metas(doc, 'xc-path')).toEqual([path]);
    expect(metas(doc, 'xc-gate')).toEqual([gate]);
    const srcs = Array.from(doc.querySelectorAll('script[src]')).map((s) => ({
      src: s.getAttribute('src'),
      type: s.getAttribute('type'),
    }));
    expect(srcs).toContainEqual({ src: script, type: 'module' });
    expect(doc.getElementById('app'), `${file}: a #app to mount into`).not.toBeNull();
  });
});

describe('V3-documents E: the entry scripts', () => {
  it.each(['./owner.tsx', './staff.tsx'])('%s renders AppShell itself and passes location.replace', (file) => {
    const src = scripts[file];
    expect(src, `web/src/screens/documents/${file.slice(2)} exists`).toBeTypeOf('string');
    expect(src).toMatch(/import\s*\{[^}]*\bAppShell\b[^}]*\}\s*from\s*['"][^'"]*AppShell['"]/);
    expect(src).toMatch(/<\s*AppShell\b/);
    expect(src).toMatch(/<\s*DocumentsScreen\b/);
    expect(src).toMatch(/location\.replace/);
  });
});

// ── through the real entry ─────────────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
}
let sent: Sent[];

const ANSWER = {
  version: 3,
  changed: true,
  data: {
    ref: 'XC-A',
    links: [
      { kind: 'passport', href: '/files/p-a', download: 'XC-A_PASSPORT.jpg' },
      { kind: 'receipt', href: '/files/r-a', download: 'XC-A_RECEIPT_FULL.html' },
    ],
  },
};
const STAFF_ANSWER = {
  version: 3,
  changed: true,
  data: {
    ref: 'XC-A',
    links: [
      { kind: 'passport', href: '/api/staff/file/p-a', download: null },
      { kind: 'receipt', href: '/api/staff/file/r-a', download: null },
    ],
  },
};

function fakeFetch(input: string | URL | Request): Promise<Response> {
  const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  const url = new URL(raw, 'http://localhost');
  sent.push({ path: url.pathname, params: url.searchParams });
  let body: unknown = { version: 1, changed: false };
  if (url.pathname === '/api/v3/views/documents') body = ANSWER;
  if (url.pathname === '/api/v3/views/staff_documents') body = STAFF_ANSWER;
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } }));
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

let app: HTMLElement;

beforeEach(() => {
  sent = [];
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  app = document.createElement('div');
  app.id = 'app';
  document.body.appendChild(app);
  setLang('th');
});

afterEach(() => {
  app.remove();
  vi.unstubAllGlobals();
  vi.resetModules();
  window.history.replaceState(null, '', '/');
});

async function mountEntry(file: './owner.tsx' | './staff.tsx', url: string): Promise<void> {
  window.history.replaceState(null, '', url);
  const load = modules[file];
  expect(load, `web/src/screens/documents/${file.slice(2)} exists`).toBeDefined();
  vi.resetModules();
  await load!();
  await settle();
}

function asked(view: string): Sent[] {
  return sent.filter((s) => s.path === `/api/v3/views/${view}`);
}

describe('V3-documents E: each page through its real entry (lane controller, 2026-10-11)', () => {
  it('/documents: the owner entry asks for the first ref and the first branch, and lists the files inside the shell', async () => {
    await mountEntry('./owner.tsx', '/documents?ref=XC-A&ref=XC-B&branch=BR-2&branch=MAIN');
    const reads = asked('documents');
    expect(reads.length, 'one read of the documents view').toBe(1);
    expect(reads[0]!.params.get('ref')).toBe('XC-A');
    expect(reads[0]!.params.getAll('ref')).toEqual(['XC-A']);
    expect(reads[0]!.params.get('branch')).toBe('BR-2');
    expect(app.querySelector('.xc-shell'), 'inside AppShell').not.toBeNull();
    expect([...app.querySelectorAll<HTMLElement>('[data-kind]')].map((e) => e.dataset.kind)).toEqual([
      'passport',
      'receipt',
    ]);
    expect(app.querySelector('a[download="XC-A_PASSPORT.jpg"]')).not.toBeNull();
  });

  it('/staff/documents: the staff entry asks staff_documents for the first ref, sends no branch, and lists the files', async () => {
    await mountEntry('./staff.tsx', '/staff/documents?ref=XC-A&ref=XC-B&branch=BR-2');
    const reads = asked('staff_documents');
    expect(reads.length, 'one read of the staff_documents view').toBe(1);
    expect(reads[0]!.params.get('ref')).toBe('XC-A');
    expect(reads[0]!.params.get('branch') ?? '', 'the staff page never names a branch').toBe('');
    expect(asked('documents').length, 'not the owner view').toBe(0);
    expect(app.querySelector('.xc-shell'), 'inside AppShell').not.toBeNull();
    expect([...app.querySelectorAll<HTMLElement>('[data-kind]')].map((e) => e.dataset.kind)).toEqual([
      'passport',
      'receipt',
    ]);
    expect(app.querySelector('a[href="/api/staff/file/p-a"]')).not.toBeNull();
    expect(app.querySelectorAll('a[download]').length).toBe(0);
  });
});
