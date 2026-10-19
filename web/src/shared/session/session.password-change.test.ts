// W5, forced password change on every page (survey xchange#564 issuecomment-6262359159 row 10): a session
// whose password must change gets `403 passwordChangeRequired` from every route but change-password,
// who-am-I and logout (`api/src/presentation/http/v3/caller.rs:59-76`; design
// `docs/design/blueprint/user-management-rbac.md:112` Q5 = A, `:299` W5). Only `/login` reads
// `mustChangePassword` today (`web/src/screens/login/LoginScreen.tsx:43`, `:71`), so a flagged session that
// opens `/admin` or `/desk` sees refusals and is never sent to the change page.
// Written before the implementation by the sub-controller (claude/dell-claude-3). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// The API this test pins, in `web/src/shared/session/session.ts` ([required]):
//   export function needsPasswordChange(status: number, code: string | undefined): boolean;
//       true only for status 403 with code `passwordChangeRequired`;
//   export function passwordChangeUrl(here: string): string;
//       `/login?next=` + encodeURIComponent(here) — `/login` shows ChangePassword for a flagged session
//       (LoginScreen.tsx:43) and `safeNext` (`screens/login/home.ts:18`) guards the return;
//   endingFetch(inner, onEnd, onPasswordChange?)  — a third, optional callback: on any request (a view
//       or a command) answered 403 `passwordChangeRequired` it calls `onPasswordChange()` once per answer
//       and never `onEnd()`; the response still reaches the caller unchanged.
//   `sharedApi()` passes a callback that does `window.location.assign(passwordChangeUrl(<path+search+hash>))`.
// The functions are read through a namespace import, so a missing export fails by result, not by compile.
import { describe, expect, it, vi } from 'vitest';
import type { Fetch } from '../api/client';
import * as session from './session';

type EndingFetch3 = (inner: Fetch, onEnd: () => void, onPasswordChange?: () => void) => Fetch;

function refusal(status: number, code: string): Response {
  return new Response(JSON.stringify({ error: { code, message: `${code} (test)` } }), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function inner(res: () => Response): Fetch {
  return async () => res();
}

describe('W5: needsPasswordChange (caller.rs:70)', () => {
  it('is exported by session.ts', () => {
    expect(typeof (session as Record<string, unknown>).needsPasswordChange).toBe('function');
  });

  it('is true only for 403 passwordChangeRequired', () => {
    const f = (session as Record<string, unknown>).needsPasswordChange as
      | ((s: number, c: string | undefined) => boolean)
      | undefined;
    expect(f, 'needsPasswordChange exists').toBeTypeOf('function');
    expect(f!(403, 'passwordChangeRequired')).toBe(true);
    expect(f!(403, 'forbidden')).toBe(false);
    expect(f!(403, undefined)).toBe(false);
    expect(f!(401, 'passwordChangeRequired')).toBe(false);
    expect(f!(200, 'passwordChangeRequired')).toBe(false);
  });
});

describe('W5: endingFetch sends a flagged session to change its password', () => {
  const ending = session.endingFetch as unknown as EndingFetch3;

  it('a view answered 403 passwordChangeRequired calls onPasswordChange once, never onEnd, and returns the answer', async () => {
    const onEnd = vi.fn();
    const onChange = vi.fn();
    const f = ending(inner(() => refusal(403, 'passwordChangeRequired')), onEnd, onChange);
    const res = await f('/api/v3/views/admin_overview?branch=MAIN', { method: 'GET' });
    expect(res.status).toBe(403);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onEnd).not.toHaveBeenCalled();
  });

  it('a command answered 403 passwordChangeRequired calls onPasswordChange too', async () => {
    const onEnd = vi.fn();
    const onChange = vi.fn();
    const f = ending(inner(() => refusal(403, 'passwordChangeRequired')), onEnd, onChange);
    await f('/api/rate-drafts', { method: 'POST', body: '{}' });
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onEnd).not.toHaveBeenCalled();
  });

  it('another 403 calls neither', async () => {
    const onEnd = vi.fn();
    const onChange = vi.fn();
    const f = ending(inner(() => refusal(403, 'forbidden')), onEnd, onChange);
    await f('/api/v3/views/admin_overview', { method: 'GET' });
    expect(onChange).not.toHaveBeenCalled();
    expect(onEnd).not.toHaveBeenCalled();
  });

  it('kept: a 401 on a view still ends the session and is not a password change', async () => {
    const onEnd = vi.fn();
    const onChange = vi.fn();
    const f = ending(inner(() => refusal(401, 'signIn')), onEnd, onChange);
    await f('/api/v3/views/me', { method: 'GET' });
    expect(onEnd).toHaveBeenCalledTimes(1);
    expect(onChange).not.toHaveBeenCalled();
  });

  it('kept: with no third callback, a 403 passwordChangeRequired does not throw and ends nothing', async () => {
    const onEnd = vi.fn();
    const f = session.endingFetch(inner(() => refusal(403, 'passwordChangeRequired')), onEnd);
    const res = await f('/api/v3/views/admin_overview', { method: 'GET' });
    expect(res.status).toBe(403);
    expect(onEnd).not.toHaveBeenCalled();
  });
});

describe('W5: passwordChangeUrl', () => {
  it('is /login with the current place as next', () => {
    const f = (session as Record<string, unknown>).passwordChangeUrl as ((here: string) => string) | undefined;
    expect(f, 'passwordChangeUrl exists').toBeTypeOf('function');
    expect(f!('/ui/admin/#rates')).toBe('/login?next=%2Fui%2Fadmin%2F%23rates');
    expect(f!('/desk?branch=MAIN')).toBe('/login?next=%2Fdesk%3Fbranch%3DMAIN');
  });
});
