// #1860 (Member.tsx half): C05 reads `my_membership` through the shared client's bare-view read,
// `ApiClient.viewBare` (#1892), and the stopgap copy of the client goes (FE controller's ruling, #1649
// issuecomment-6181132055). Written before the implementation; do not edit, delete or weaken.
//
// The stopgap and `ApiClient` send the same request and parse errors the same way, so no screen output tells them
// apart: these tests pin the wiring (the screen calls `viewBare('my_membership')`) and that the copy is gone.
// The screen's behaviour stays pinned by `Member.test.tsx` and `worded.test.tsx`, which must pass unchanged.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { MemberScreen } from './Member';
import * as table from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  join(...segments: string[]): string;
  dirname(p: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';
let fs: NodeFs;
let path: NodePath;
let thisDir: string;

const MEMBER = {
  tier: 'GOLD',
  effectiveUntil: '2026-11-15T00:00:00Z',
  points: { available: 100, pending: 25 },
  pointsUnavailable: false,
  nextExpiry: null,
  progress: null,
};

type BareRead = (name: string, q?: { branch?: string }) => Promise<unknown>;

describe('#1860: Member.tsx reads my_membership through ApiClient.viewBare', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeAll(async () => {
    fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
    thisDir = path.dirname(new URL(import.meta.url).pathname);
  });

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setAudience('customer');
    setLang('th');
    // Anything that reaches the network is a failure of this file's premise: the read goes through viewBare.
    vi.stubGlobal('fetch', async () => {
      throw new Error('Member.tsx must not call fetch itself; it reads through ApiClient.viewBare');
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function settle(): Promise<void> {
    await new Promise((r) => setTimeout(r, 20));
  }

  function spyViewBare(impl: BareRead) {
    const proto = ApiClient.prototype as unknown as { viewBare: BareRead };
    return vi.spyOn(proto, 'viewBare').mockImplementation(impl);
  }

  it('1. calls ApiClient.viewBare("my_membership") and renders what it answers', async () => {
    const spy = spyViewBare(async () => MEMBER);
    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(spy).toHaveBeenCalled();
    expect(spy.mock.calls[0][0]).toBe('my_membership');
    expect(host.textContent).toContain('GOLD');
  });

  it('2. an outOfScope refusal from viewBare shows the worded outOfScope line', async () => {
    spyViewBare(async () => {
      throw new ApiError({ code: 'outOfScope', message: 'Forbidden', status: 403 });
    });
    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.textContent).toContain(table.th.outOfScope);
  });

  it('3. the stopgap copy of the client is gone from Member.tsx', () => {
    const source = fs.readFileSync(path.join(thisDir, 'Member.tsx'), 'utf8');
    expect(source, 'no raw fetch').not.toMatch(/\bfetch\(/);
    expect(source, 'no copied error parser').not.toContain('parseErrorResponse');
    expect(source, 'no stopgap pass-through').not.toContain('normalizeMembershipData');
    expect(source, 'no stopgap label left').not.toContain('stopgap (#1860)');
  });
});
