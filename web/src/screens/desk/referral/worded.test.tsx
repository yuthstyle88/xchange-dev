import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import { ReferralCodeField } from './ReferralCodeField';
import * as texts from './i18n';

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

describe('ReferralCodeField wordedError (#1781)', () => {
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

  function setupWithCommandError(errorToThrow: unknown) {
    const mockApi = {
      view: vi.fn(async () => ({ data: { permissions: ['ReferralAttach'] } })),
      command: vi.fn().mockRejectedValue(errorToThrow),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-worded-1" api={mockApi} />, host);
    return { mockApi };
  }

  async function triggerSubmit(code = 'TESTCODE') {
    await flush();
    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = code;
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
    button.click();
    await flush();
    await new Promise((r) => setTimeout(r, 0));
  }

  function alertText(): string {
    const el = host.querySelector<HTMLElement>('[role="alert"]');
    return (el?.textContent ?? '').trim();
  }

  describe('handleSubmit error catch', () => {
    it('(1) network TypeError shows error.network and reacts to setLang without remounting', async () => {
      setupWithCommandError(new TypeError('Failed to fetch'));
      await triggerSubmit();

      expect(alertText()).toBe(sharedTh['error.network']);

      setLang('en');
      await flush();
      expect(alertText()).toBe(sharedEn['error.network']);
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      setupWithCommandError(
        new ApiError({
          status: 500,
          code: 'roleNotAllowedHere',
          message: 'role not allowed to attach referral',
        }),
      );
      await triggerSubmit();

      expect(alertText()).toBe(sharedTh['error.serverError']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      setupWithCommandError(
        new ApiError({
          status: 400,
          code: 'unmappedReferralCodeRefusal',
          message: 'รหัสแนะนำนี้หมดอายุแล้วจากระบบต้นทาง',
        }),
      );
      await triggerSubmit();

      expect(alertText()).toBe('รหัสแนะนำนี้หมดอายุแล้วจากระบบต้นทาง');
    });

    it('WE4 screen-specific code: 409 referralAlreadyCaptured still shows alreadyCaptured and locks', async () => {
      setupWithCommandError(
        new ApiError({
          status: 409,
          code: 'referralAlreadyCaptured',
          message: 'conflict',
        }),
      );
      await triggerSubmit();

      expect(alertText()).toBe(texts.th.alreadyCaptured);
      const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
      const button = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
      expect(input.disabled).toBe(true);
      expect(button.disabled).toBe(true);
    });
  });
});
