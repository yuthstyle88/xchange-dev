// XC Counter CA-5 (docs/design/xc-counter-app.md §5.4): "The app re-binds after every sign-in and on each desk load".
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//
// The app emits no sign-in event: `/login` navigates, so a sign-in is a new desk load, and inside a loaded desk the
// signal is `session.me()` (see binding.ts).
//
// | # | Case | Required |
// |---|---|---|
// | C1 | desk load, signed in, in the app | one bind |
// | C2 | desk load in a plain browser | no request, ever, even when `me` changes |
// | C3 | `me` unknown at load, then known | no bind until known, then one |
// | C4 | sign-out, then sign-in again in this page | binds again |
// | C5 | `me` becomes another account | binds again |
// | C6 | a poll answers the same account | no second bind |
// | C7 | the server refuses | nothing thrown, no retry |
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import { ApiClient, type Fetch } from '../api/client';
import type { Me } from '../api/generated/common';
import { makeMe } from '../testing/fixtures/common';
import { useCounterBinding } from './binding';

const SOMCHAI = makeMe({ id: 'somchai' });
const SUDA = makeMe({ id: 'suda' });
const BIND = '/api/v3/commands/desk_bind_counter';

let dispose: (() => void) | undefined;

function setup(first: Me | undefined, status = 200) {
  const calls: string[] = [];
  const fetchFn: Fetch = async (input) => {
    calls.push(input);
    return status === 200
      ? new Response(JSON.stringify({ counterAgentId: 'agent-1' }), { status })
      : new Response(JSON.stringify({ error: { code: 'NONCE_INVALID', message: 'no' } }), { status });
  };
  const [me, setMe] = createSignal<Me | undefined>(first);
  createRoot((d) => {
    dispose = d;
    useCounterBinding(new ApiClient(fetchFn), { me });
  });
  return { calls, setMe };
}

function inTheApp() {
  const bindingNonce = vi.fn(async () => 'v1.nonce');
  window.xcCounter = {
    info: vi.fn(async () => {
      throw new Error('info is not part of this test');
    }),
    scanPassport: vi.fn(async () => ({ ok: false as const, reason: 'noScanner' as const })),
    bindingNonce,
  };
  return bindingNonce;
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  delete window.xcCounter;
});

describe('useCounterBinding', () => {
  it('C1: binds once on a desk load, signed in, inside the app', async () => {
    const bindingNonce = inTheApp();
    const { calls } = setup(SOMCHAI);
    await settle();
    expect(bindingNonce).toHaveBeenCalledTimes(1);
    expect(calls).toEqual([BIND]);
  });

  it('C2: does nothing in a plain browser', async () => {
    const { calls, setMe } = setup(SOMCHAI);
    await settle();
    setMe(SUDA);
    await settle();
    expect(calls).toEqual([]);
  });

  it('C3: waits for me, then binds once', async () => {
    inTheApp();
    const { calls, setMe } = setup(undefined);
    await settle();
    expect(calls, 'nobody is known yet').toEqual([]);
    setMe(SOMCHAI);
    await settle();
    expect(calls).toEqual([BIND]);
  });

  it('C4: binds again after a sign-out and a sign-in in the same page', async () => {
    inTheApp();
    const { calls, setMe } = setup(SOMCHAI);
    await settle();
    setMe(undefined);
    await settle();
    expect(calls, 'signing out binds nothing').toEqual([BIND]);
    setMe(SOMCHAI);
    await settle();
    expect(calls).toEqual([BIND, BIND]);
  });

  it('C5: binds again when me becomes another account', async () => {
    inTheApp();
    const { calls, setMe } = setup(SOMCHAI);
    await settle();
    setMe(SUDA);
    await settle();
    expect(calls).toEqual([BIND, BIND]);
  });

  it('C6: a poll that answers the same account does not bind again', async () => {
    inTheApp();
    const { calls, setMe } = setup(SOMCHAI);
    await settle();
    setMe({ ...SOMCHAI, name: 'renamed' });
    await settle();
    expect(calls).toEqual([BIND]);
  });

  it('C7: a refusal throws nothing and is not retried', async () => {
    inTheApp();
    // vitest fails the run on an unhandled rejection, so a throw out of the hook cannot pass here.
    const { calls } = setup(SOMCHAI, 409);
    await settle();
    expect(calls).toEqual([BIND]);
    await settle();
    expect(calls, 'no retry').toEqual([BIND]);
  });
});
