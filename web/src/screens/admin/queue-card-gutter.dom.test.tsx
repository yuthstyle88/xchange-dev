// #4802 (#4302 rework): DOM-level test proving that the gutter selector in admin.css
// matches the mounted admin shell's `.admin-main` when the floating queue card renders
// via <Portal> under <body>.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient, ViewAnswer, ViewParams } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../shared/testing/page-wide';
import { meOf } from '../../test-support/me';
import { AdminShell } from './AdminShell';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  join(...segments: string[]): string;
  dirname(p: string): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const norm = (s: string) => s.replace(/\s+/g, '');
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1').replace(/\)\s*(?=[.#\[:a-zA-Z])/g, ') ');
  const block = (body: string, media: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const k = d.indexOf(':');
        if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
      }
      for (const sel of m[1]!.split(',')) out.push({ media, selector: normSel(sel), decls });
    }
  };
  let rest = '';
  let i = 0;
  while (i < src.length) {
    const at = src.indexOf('@media', i);
    if (at < 0) {
      rest += src.slice(i);
      break;
    }
    rest += src.slice(i, at);
    const open = src.indexOf('{', at);
    let depth = 1;
    let j = open + 1;
    while (j < src.length && depth > 0) {
      if (src[j] === '{') depth += 1;
      if (src[j] === '}') depth -= 1;
      j += 1;
    }
    block(src.slice(open + 1, j - 1), norm(src.slice(at + 6, open)));
    i = j;
  }
  block(rest, '');
  return out;
}

async function readAdminRules(): Promise<Rule[]> {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const file = path.join(path.dirname(new URL(import.meta.url).pathname), 'admin.css');
  return parseCss(fs.readFileSync(file, 'utf8'));
}

const managerMe = meOf({
  id: 'mgr',
  role: 'BRANCH_MANAGER',
  branch: 'MAIN',
  name: 'Manager',
  branchName: 'สำนักงานใหญ่',
  branches: [{ id: 'MAIN', code: 'MAIN', name: 'สำนักงานใหญ่' }],
});

const fakeApi = {
  async view<T>(name: string, _q: ViewParams = {}): Promise<ViewAnswer<T>> {
    if (name === 'me') return { version: 3, changed: true, data: managerMe as unknown as T };
    if (name === 'admin_branches') {
      return { version: 3, changed: true, data: { branches: [{ id: 'MAIN', name: 'สำนักงานใหญ่' }] } as unknown as T };
    }
    throw new Error(`unexpected view ${name}`);
  },
} as unknown as ApiClient;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

beforeEach(() => {
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const url = String(input);
      if (url.includes('/health/ready')) {
        return jsonResponse({ version: '3.0.0', build: 'abcdef1234567', channel: 'staging', builtAt: '2026-10-08T00:00:00Z' });
      }
      if (url.startsWith('/api/v3/views/me')) {
        return jsonResponse({ version: 1, changed: true, data: managerMe });
      }
      if (url.startsWith(QUEUE_CARD_PATH)) {
        return jsonResponse(queueCardUnchanged());
      }
      return jsonResponse({}, 404);
    }),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  host = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

describe('#4802 admin queue card gutter DOM selector match', () => {
  it('gutter rules in admin.css match the mounted .admin-main while QueueCard renders in a body Portal', async () => {
    host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <AdminShell api={fakeApi} menus={[]} />, host);
    await settle();

    const card = document.body.querySelector('.queue-card');
    expect(card, '.queue-card is mounted via Portal under body').not.toBeNull();
    expect(host.contains(card), '.queue-card is outside the AdminShell host container').toBe(false);

    const adminMain = host.querySelector('.admin-main');
    expect(adminMain, '.admin-main is mounted').not.toBeNull();

    const rules = await readAdminRules();
    const desktopSelectors = rules
      .filter((r) => r.media === '(min-width:1024px)' && r.selector.includes('.admin-main') && r.decls.has('padding-right'))
      .map((r) => r.selector);
    const mobileSelectors = rules
      .filter((r) => r.media === '(max-width:1023px)' && r.selector.includes('.admin-main') && r.decls.has('padding-bottom'))
      .map((r) => r.selector);

    expect(desktopSelectors.length, 'desktop gutter selector exists').toBeGreaterThan(0);
    expect(mobileSelectors.length, 'mobile gutter selector exists').toBeGreaterThan(0);
    expect(desktopSelectors.some((sel) => adminMain!.matches(sel)), 'desktop gutter selector matches mounted .admin-main').toBe(true);
    expect(mobileSelectors.some((sel) => adminMain!.matches(sel)), 'mobile gutter selector matches mounted .admin-main').toBe(true);

    // When the queue card is removed, no gutter selector matches .admin-main.
    card!.remove();
    expect(desktopSelectors.some((sel) => adminMain!.matches(sel)), 'no desktop gutter without .queue-card').toBe(false);
    expect(mobileSelectors.some((sel) => adminMain!.matches(sel)), 'no mobile gutter without .queue-card').toBe(false);
  });
});
