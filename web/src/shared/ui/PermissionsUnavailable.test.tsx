// G-ME, GME-0 (`docs/design/web-silent-failures.md` §5): the one shared worded line for a failed `me` read.
// Written before the implementation by the ticket author (claude/e8020026's sub-agent). Do not edit, delete or weaken;
// add tests in a file of your own.
//
// The contract (header of `PermissionsUnavailable.tsx`, from the skeleton):
//   - error null or undefined ⇒ nothing rendered;
//   - otherwise `<p role="alert" class="xc-perm-unavailable" data-permissions-unavailable>` with
//     `permissionsUnavailableText(error)` = `t('shell.permissions.unavailable')` + ' ' + `wordedError(error)`, and one
//     `button[data-action="permissions-retry"]` reading `t('shell.permissions.retry')`, disabled while `busy`, whose
//     click calls `onRetry()` once;
//   - wired to the session store (`error={session.meError()}`, `onRetry={() => session.retryMe()}`), a retry re-reads
//     `me` and the line goes away on a good answer.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ViewAnswer, type ViewParams } from '../api/client';
import { setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import { wordedError } from '../i18n/worded-error';
import { createSessionStore } from '../session/session';
import type { Me } from '../testing/fixtures';
import { meOf } from '../../test-support/me';
import { PermissionsUnavailable, permissionsUnavailableText } from './PermissionsUnavailable';

const down = () => new ApiError({ code: 'serverError', message: 'database down', status: 500 });
const network = () => new ApiError({ code: 'network', message: 'Failed to fetch', status: 0 });

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

const line = () => host.querySelector<HTMLElement>('[data-permissions-unavailable]');
const retry = () => host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
  setLang('th');
});

describe('permissionsUnavailableText(error)', () => {
  it('is the shared unavailable line, one space, then the worded error', () => {
    const err = down();
    expect(permissionsUnavailableText(err)).toBe(`${th['shell.permissions.unavailable']} ${wordedError(err)}`);
  });

  it('a network failure and a server failure read differently', () => {
    expect(permissionsUnavailableText(network())).not.toBe(permissionsUnavailableText(down()));
    expect(permissionsUnavailableText(new TypeError('Failed to fetch'))).toContain(th['shell.permissions.unavailable']);
  });

  it('follows the language', () => {
    setLang('en');
    expect(permissionsUnavailableText(down())).toContain(en['shell.permissions.unavailable']);
  });
});

describe('PermissionsUnavailable', () => {
  it('renders nothing while there is no error (null or undefined)', () => {
    dispose = render(
      () => (
        <>
          <PermissionsUnavailable error={null} onRetry={() => {}} />
          <PermissionsUnavailable error={undefined} onRetry={() => {}} />
        </>
      ),
      host,
    );
    expect(host.innerHTML).toBe('');
  });

  it('with an error: one role="alert" line, worded, with the retry button', () => {
    const err = down();
    dispose = render(() => <PermissionsUnavailable error={err} onRetry={() => {}} />, host);
    const p = line();
    expect(p).not.toBeNull();
    expect(p!.tagName).toBe('P');
    expect(p!.getAttribute('role')).toBe('alert');
    expect(p!.classList.contains('xc-perm-unavailable')).toBe(true);
    expect(p!.textContent).toContain(permissionsUnavailableText(err));
    expect(p!.textContent).not.toContain('database down');
    expect(p!.textContent).not.toContain('serverError');
    const b = retry();
    expect(b).not.toBeNull();
    expect(p!.contains(b)).toBe(true);
    expect(b!.textContent).toBe(th['shell.permissions.retry']);
    expect(b!.disabled).toBe(false);
    expect(b!.classList.contains('xc-perm-unavailable-retry')).toBe(true);
  });

  it('the retry calls onRetry once per click', () => {
    const onRetry = vi.fn();
    dispose = render(() => <PermissionsUnavailable error={down()} onRetry={onRetry} />, host);
    retry()!.click();
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('busy disables the retry; a click then calls nothing', () => {
    const onRetry = vi.fn();
    dispose = render(() => <PermissionsUnavailable error={down()} onRetry={onRetry} busy />, host);
    expect(retry()!.disabled).toBe(true);
    retry()!.click();
    expect(onRetry).not.toHaveBeenCalled();
  });

  it('is reactive: the line appears and goes with the error, and follows setLang', () => {
    const [err, setErr] = createSignal<ApiError | null>(null);
    dispose = render(() => <PermissionsUnavailable error={err()} onRetry={() => {}} />, host);
    expect(line()).toBeNull();
    setErr(down());
    expect(line()).not.toBeNull();
    setLang('en');
    expect(line()!.textContent).toContain(en['shell.permissions.unavailable']);
    expect(retry()!.textContent).toBe(en['shell.permissions.retry']);
    setErr(null);
    expect(line()).toBeNull();
  });
});

describe('wired to the session store: the retry re-reads me (GME-0)', () => {
  const teller: Me = meOf({ id: 'e-7', role: 'TELLER', branch: 'XC-B01', name: null, branchName: 'XC-B01', branches: [] });

  it('me fails ⇒ the line; retry ⇒ one more me read; a good answer ⇒ the line goes', async () => {
    vi.useFakeTimers({ now: 0 });
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
    const script: Array<ViewAnswer<Me> | Error> = [down(), { version: 1, changed: true, data: teller }];
    const names: string[] = [];
    let i = 0;
    const api = {
      post: async <B, R>(_p: string, _b: B): Promise<R> => ({}) as R,
      view: async <U,>(name: string, _params: ViewParams = {}): Promise<ViewAnswer<U>> => {
        names.push(name);
        const next = script[Math.min(i++, script.length - 1)]!;
        if (next instanceof Error) throw next;
        return next as never;
      },
    };
    const session = createSessionStore(api);
    dispose = render(
      () => <PermissionsUnavailable error={session.meError()} onRetry={() => session.retryMe()} />,
      host,
    );
    await vi.advanceTimersByTimeAsync(0);
    expect(names).toEqual(['me']);
    expect(line(), 'the failed me read is worded').not.toBeNull();

    retry()!.click();
    await vi.advanceTimersByTimeAsync(0);
    expect(names).toEqual(['me', 'me']);
    expect(line(), 'the good answer clears the line').toBeNull();
    expect(session.me()).toBe(teller);
    session.dispose();
  });
});
