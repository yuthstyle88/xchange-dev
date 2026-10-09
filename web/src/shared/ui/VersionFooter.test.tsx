// V1-auth-layout (#531; design docs/design/xchange-v3-web.md §12.21.1): AppShell's version footer moved into its
// own file, so the signed-out frame (AuthLayout) and AppShell show the same line from one place (♻️ one shared core).
// Written before the implementation by the lane controller (claude/608dff). Do not edit, delete or weaken.
//
// What this test pins, in `web/src/shared/ui/VersionFooter.tsx`:
//
//   export function VersionFooter(): JSX.Element;
//
// - Its `<footer>` is byte-identical to the one AppShell renders for the same GET /health/ready answer
//   (`v{version} · {build, 7} · {channel}` with builtAt beside it, #293), for an answer with identity, a 503 that
//   still carries identity, an answer missing a field, and an unreadable answer.
// - AppShell renders it instead of its own copy: AppShell.tsx imports VersionFooter and no longer reads /health/ready.
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { AppShell } from './AppShell';

type VersionFooterFn = () => JSX.Element;

const modules = import.meta.glob<{ VersionFooter?: VersionFooterFn }>('./VersionFooter.tsx', { eager: true });

function VersionFooterOrFail(): VersionFooterFn {
  const mod = modules['./VersionFooter.tsx'];
  expect(mod?.VersionFooter, 'web/src/shared/ui/VersionFooter.tsx exports VersionFooter (§12.21)').toBeTypeOf(
    'function',
  );
  return mod!.VersionFooter!;
}

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let appShellSource = '';
const disposers: Array<() => void> = [];

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  appShellSource = fs.readFileSync(dir + 'AppShell.tsx', 'utf8');
});

afterEach(() => {
  while (disposers.length) disposers.pop()!();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

/** Every /health/ready read answers `answer()`; anything else (AppShell's me read) answers 503. */
function stubReady(answer: () => Promise<Response>) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string) =>
      /\/health\/ready$/.test(String(url)) ? answer() : new Response('{}', { status: 503 }),
    ),
  );
}

async function footers(): Promise<{ shell: HTMLElement; own: HTMLElement }> {
  const VersionFooter = VersionFooterOrFail();
  const a = document.createElement('div');
  const b = document.createElement('div');
  document.body.append(a, b);
  disposers.push(
    render(
      () => (
        <AppShell title="t">
          <p>body</p>
        </AppShell>
      ),
      a,
    ),
  );
  disposers.push(render(() => <VersionFooter />, b));
  await settle();
  const shell = a.querySelectorAll('footer');
  const own = b.querySelectorAll('footer');
  expect(shell, "AppShell's footer").toHaveLength(1);
  expect(own, "VersionFooter's footer").toHaveLength(1);
  return { shell: shell[0] as HTMLElement, own: own[0] as HTMLElement };
}

describe('VersionFooter — the same output as AppShell (§12.21.1)', () => {
  it('a full identity: the build line, byte-identical to AppShell', async () => {
    stubReady(async () => new Response(JSON.stringify(READY), { status: 200 }));
    const { shell, own } = await footers();
    expect(own.textContent).toContain('v1.0.0 · 0123456 · staging');
    expect(own.textContent).toContain('2026-10-08T01:02:03Z');
    expect(own.outerHTML).toBe(shell.outerHTML);
  });

  it('a 503 that still carries identity: the build line, byte-identical to AppShell', async () => {
    const body = { ...READY, status: 'degraded', database: false };
    stubReady(async () => new Response(JSON.stringify(body), { status: 503 }));
    const { shell, own } = await footers();
    expect(own.textContent).toContain('v1.0.0 · 0123456 · staging');
    expect(own.outerHTML).toBe(shell.outerHTML);
  });

  it('an answer missing builtAt: an empty footer, byte-identical to AppShell', async () => {
    const partial: Record<string, unknown> = { ...READY };
    delete partial.builtAt;
    stubReady(async () => new Response(JSON.stringify(partial), { status: 200 }));
    const { shell, own } = await footers();
    expect(own.textContent).toBe('');
    expect(own.outerHTML).toBe(shell.outerHTML);
  });

  it('an unreadable answer: an empty footer, no "undefined", byte-identical to AppShell', async () => {
    stubReady(async () => Promise.reject(new TypeError('offline')));
    const { shell, own } = await footers();
    expect(own.textContent ?? '').not.toContain('undefined');
    expect(own.outerHTML).toBe(shell.outerHTML);
  });
});

describe('AppShell renders VersionFooter (§12.21.3)', () => {
  it('AppShell.tsx imports VersionFooter and no longer reads /health/ready itself', () => {
    expect(appShellSource).toMatch(/import\s*\{\s*VersionFooter\s*\}\s*from\s*['"]\.\/VersionFooter['"]/);
    expect(appShellSource).toContain('<VersionFooter');
    expect(appShellSource).not.toContain('/health/ready');
  });
});
