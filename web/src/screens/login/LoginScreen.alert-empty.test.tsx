// OL-7 row 1 re-draft on feat/one-look-7 (VERIFY.md on main 4a577dac: still NOT FIXED). Same test as test/ol6-b-login-alert
// (bd01bea3, ticket #6697, never done). Written before the fix by claude/studio-claude-17. Do not edit, delete or weaken.
// OL-6 B login-alert: `.xc-auth #message:empty { display: none }` only hides the line if Solid leaves it with no child
// node while there is no message. Pins that, so a later markup change (a space, a wrapper) cannot bring the box back
// empty.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import type { AuthApi } from './auth-api';
import { LoginScreen } from './LoginScreen';

const disposers: Array<() => void> = [];

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
}

describe('LoginScreen #message', () => {
  beforeEach(() => {
    setLang('th');
    vi.stubGlobal('fetch', async () => new Response('{}', { status: 503, headers: { 'content-type': 'application/json' } }));
  });

  afterEach(() => {
    for (const d of disposers.splice(0)) d();
    vi.unstubAllGlobals();
  });

  it('LA-3 with no message, #message has no child node (so :empty hides it)', async () => {
    const c = document.createElement('div');
    document.body.appendChild(c);
    const auth = {
      session: async () => ({ user: null }),
      login: async () => {
        throw new Error('login was not expected in this test');
      },
    } as unknown as AuthApi;
    disposers.push(render(() => <LoginScreen auth={auth} navigate={() => {}} />, c));
    disposers.push(() => c.remove());
    await flush();
    const m = c.querySelector('#message');
    expect(m, '#message is rendered').not.toBeNull();
    expect(m!.getAttribute('role')).toBe('alert');
    expect(m!.childNodes.length).toBe(0);
    expect(m!.matches(':empty')).toBe(true);
  });
});
