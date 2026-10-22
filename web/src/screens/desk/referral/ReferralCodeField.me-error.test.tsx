// G-ME, GME-F (`docs/design/web-silent-failures.md` §5): the desk referral field when its `me` read fails.
// Written before the implementation by the ticket author (claude/e8020026's sub-agent). Do not edit, delete or weaken;
// add tests in a file of your own. The harness (a mock `api`) is copied from ReferralCodeField.permission.test.tsx.
//
// What changes (the #1719 standard, replacing the retired "when me fails, the field is hidden" row): the field keeps
// reading `me` through its `api` prop (pinned by the permission test), and when that read fails:
//   - the field is on screen (`.desk-referral-field`), with the shared line `[data-permissions-unavailable]`
//     (role="alert") inside it;
//   - the input and the save button are **disabled**; the button's `title` is `permissionsUnavailableText(error)`;
//     nothing can be sent;
//   - the line's retry (`button[data-action="permissions-retry"]`) calls `api.view('me')` once more; a good answer
//     removes the line and the field follows the permission as today.
// What stays (guards): with `me` loaded and no `ReferralAttach`, nothing renders (`ReferralCodeField.tsx:84`,
// innerHTML ''); with it, the field is enabled and no line shows.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as shared } from '../../../shared/i18n/th';
import { ReferralCodeField } from './ReferralCodeField';

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
  setLang('th');
});

/** A mock api whose `me` answers come from `answers` in order (the last repeats); an Error is thrown. */
function apiWith(answers: Array<{ permissions: string[] } | Error>) {
  let i = 0;
  return {
    view: vi.fn(async (name: string) => {
      if (name !== 'me') throw new Error(`unexpected view: ${name}`);
      const next = answers[Math.min(i++, answers.length - 1)]!;
      if (next instanceof Error) throw next;
      return { version: 1, changed: true, data: next };
    }),
    command: vi.fn().mockResolvedValue({ captureId: 'cap-1', status: 'ACTIVE' }),
  };
}

const offline = () => new TypeError('Failed to fetch');
const down = () => new ApiError({ status: 500, code: 'internalError', message: 'internal error' });
const unauthorized = () => new ApiError({ status: 401, code: 'unauthorized', message: 'unauthorized' });

const field = () => host.querySelector<HTMLElement>('.desk-referral-field');
const input = () => host.querySelector<HTMLInputElement>('.desk-referral-input');
const save = () => host.querySelector<HTMLButtonElement>('.desk-referral-btn');
const line = () => host.querySelector<HTMLElement>('[data-permissions-unavailable]');
const retry = () => host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]');

async function mount(api: ReturnType<typeof apiWith>, tx = 'tx-200') {
  dispose = render(() => <ReferralCodeField transactionId={tx} api={api as unknown as ApiClient} />, host);
  await flush();
}

function type(code: string) {
  const el = input()!;
  el.value = code;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('G-ME: me fails ⇒ the field shows disabled, with the shared worded line', () => {
  for (const [label, make] of [
    ['a network TypeError', offline],
    ['a 500', down],
    ['a 401', unauthorized],
  ] as const) {
    it(`${label}: field present, input and save disabled, the reason as title, the line inside`, async () => {
      const api = apiWith([make()]);
      await mount(api);
      expect(api.view).toHaveBeenCalledWith('me');
      expect(field(), 'the field is not hidden').not.toBeNull();
      expect(input()!.disabled).toBe(true);
      expect(save()!.disabled).toBe(true);
      expect(save()!.title).toContain(shared['shell.permissions.unavailable']);
      const p = line();
      expect(p).not.toBeNull();
      expect(p!.getAttribute('role')).toBe('alert');
      expect(field()!.contains(p)).toBe(true);
      expect(p!.textContent).toContain(shared['shell.permissions.unavailable']);
    });
  }

  it('the title is worded, never the raw message or code', async () => {
    await mount(apiWith([down()]));
    expect(save()!.title).toBe(`${shared['shell.permissions.unavailable']} ${shared['error.serverError']}`);
    expect(host.textContent).not.toContain('internal error');
    expect(host.textContent).not.toContain('internalError');
  });

  it('nothing can be sent while the permissions are unknown', async () => {
    const api = apiWith([down()]);
    await mount(api);
    type('TELLER88');
    save()!.click();
    await flush();
    expect(api.command).not.toHaveBeenCalled();
  });

  it('retry calls api.view("me") once more; with ReferralAttach the line goes and the field works', async () => {
    const api = apiWith([offline(), { permissions: ['ReferralAttach'] }]);
    await mount(api, 'tx-201');
    expect(api.view).toHaveBeenCalledTimes(1);
    retry()!.click();
    await flush();
    expect(api.view).toHaveBeenCalledTimes(2);
    expect(api.view).toHaveBeenLastCalledWith('me');
    expect(line()).toBeNull();
    expect(input()!.disabled).toBe(false);
    expect(save()!.disabled).toBe(false);
    expect(save()!.title).toBe('');

    type('TELLER88');
    save()!.click();
    await flush();
    expect(api.command).toHaveBeenCalledTimes(1);
    expect(api.command).toHaveBeenCalledWith(
      'POST',
      '/api/transactions/tx-201/referral',
      { id: 'tx-201', referralCode: 'TELLER88' },
      expect.any(String),
    );
  });

  it('retry that brings a me without ReferralAttach: nothing renders, as today', async () => {
    const api = apiWith([down(), { permissions: ['BranchDayOperate'] }]);
    await mount(api);
    retry()!.click();
    await flush();
    expect(host.innerHTML).toBe('');
  });

  it('retry that fails again keeps the line and the disabled field', async () => {
    const api = apiWith([down(), offline()]);
    await mount(api);
    retry()!.click();
    await flush();
    expect(api.view).toHaveBeenCalledTimes(2);
    expect(line()).not.toBeNull();
    expect(save()!.disabled).toBe(true);
    expect(save()!.title).toBe(`${shared['shell.permissions.unavailable']} ${shared['error.network']}`);
  });
});

describe('G-ME guards: me loaded ⇒ today’s behaviour, no line', () => {
  it('with ReferralAttach: the field is enabled, no line, no title', async () => {
    await mount(apiWith([{ permissions: ['ReferralAttach'] }]));
    expect(field()).not.toBeNull();
    expect(line()).toBeNull();
    expect(input()!.disabled).toBe(false);
    expect(save()!.disabled).toBe(false);
    expect(save()!.title).toBe('');
  });

  it('without ReferralAttach: nothing renders (hidden, not disabled)', async () => {
    await mount(apiWith([{ permissions: ['BranchDayOperate'] }]));
    expect(host.innerHTML).toBe('');
  });
});
