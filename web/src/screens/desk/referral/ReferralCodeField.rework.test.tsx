import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, screenT, setLang } from '../../../shared/i18n';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import { ReferralCodeField } from './ReferralCodeField';
import * as texts from './i18n';

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

function setupWithCommandError(errorToThrow: unknown) {
  const mockApi = {
    view: vi.fn(async () => ({ data: { permissions: ['ReferralAttach'] } })),
    command: vi.fn().mockRejectedValue(errorToThrow),
  } as unknown as ApiClient;

  dispose = render(() => <ReferralCodeField transactionId="tx-rework-1" api={mockApi} />, host);
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

describe('ReferralCodeField #1704: worded error handling in handleSubmit', () => {
  it('a 500 server error shows errorText(serverError) and never raw err.message', async () => {
    setupWithCommandError(
      new ApiError({
        status: 500,
        code: 'serverError',
        message: 'Internal Server 500 Error Occurred',
      }),
    );
    await triggerSubmit();

    const alert = host.querySelector<HTMLElement>('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    expect(alert!.textContent).not.toContain('Internal Server 500 Error Occurred');
  });

  it('a refusal with a known code shows worded text from errorText, never raw code or message', async () => {
    setupWithCommandError(
      new ApiError({
        status: 403,
        code: 'roleNotAllowedHere',
        message: 'raw_backend_refusal_details',
      }),
    );
    await triggerSubmit();

    const alert = host.querySelector<HTMLElement>('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(errorText('roleNotAllowedHere', 'refused'));
    expect(alert!.textContent).not.toContain('raw_backend_refusal_details');
    expect(alert!.textContent).not.toContain('roleNotAllowedHere');
  });

  it('a refusal with an unknown code shows the screen worded error, never raw code or message', async () => {
    setupWithCommandError(
      new ApiError({
        status: 400,
        code: 'unknownCustomCode',
        message: 'raw_unworded_server_message',
      }),
    );
    await triggerSubmit();

    const alert = host.querySelector<HTMLElement>('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toBe('raw_unworded_server_message');
    expect(alert!.textContent).not.toContain('unknownCustomCode');
  });

  it('a non-ApiError shows generic worded errorText, not invalidCode or raw String(err)', async () => {
    const t = screenT('desk.referral', texts);
    setupWithCommandError(new Error('uncaught socket error trace'));
    await triggerSubmit();

    const alert = host.querySelector<HTMLElement>('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toBe(errorText('network', 'เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่'));
    expect(alert!.textContent).not.toBe(t('invalidCode'));
    expect(alert!.textContent).not.toContain('uncaught socket error trace');
  });

  it('a network TypeError shows generic worded errorText, not invalidCode', async () => {
    const t = screenT('desk.referral', texts);
    setupWithCommandError(new TypeError('Failed to fetch'));
    await triggerSubmit();

    const alert = host.querySelector<HTMLElement>('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toBe(errorText('network', 'เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่'));
    expect(alert!.textContent).not.toBe(t('invalidCode'));
    expect(alert!.textContent).not.toContain('Failed to fetch');
  });

  it('in English, worded error uses English translations', async () => {
    setLang('en');
    setupWithCommandError(
      new ApiError({
        status: 500,
        code: 'serverError',
        message: 'Server failed 500',
      }),
    );
    await triggerSubmit();

    const alert = host.querySelector<HTMLElement>('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toBe(errorText('serverError', 'Server error'));
    expect(alert!.textContent).not.toContain('Server failed 500');
  });
});
