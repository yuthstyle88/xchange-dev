import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { ReferralCodeField } from './ReferralCodeField';
import { th, en } from './i18n';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

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

function mount(commandMock = vi.fn().mockResolvedValue({ captureId: 'cap-1', status: 'ACTIVE' })) {
  const mockApi = {
    view: vi.fn(async () => ({ data: { permissions: ['ReferralAttach'] } })),
    command: commandMock,
  } as unknown as ApiClient;

  dispose = render(() => <ReferralCodeField transactionId="tx-v-1" api={mockApi} />, host);
  return { mockApi };
}

function type(val: string) {
  const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
  input.value = val;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

function clickSave() {
  const btn = host.querySelector<HTMLButtonElement>('.desk-referral-btn')!;
  btn.click();
}

function alertText(): string {
  const el = host.querySelector<HTMLElement>('[role="alert"]');
  return (el?.textContent ?? '').trim();
}

describe('ReferralCodeField input validation (#4907)', () => {
  it('V5 referral blank -> worded line; no request sent', async () => {
    const { mockApi } = mount();
    await flush();

    type('   ');
    clickSave();
    await flush();

    expect(mockApi.command).not.toHaveBeenCalled();
    expect(alertText()).toBe(th.codeRequired);
  });

  it('V6 referral ab1 -> worded line; no request sent', async () => {
    const { mockApi } = mount();
    await flush();

    type(' ab1 ');
    clickSave();
    await flush();

    expect(mockApi.command).not.toHaveBeenCalled();
    expect(alertText()).toBe(th.invalidCode);
  });

  it('V7 referral abc123 -> request sent (as today)', async () => {
    const { mockApi } = mount();
    await flush();

    type(' abc123 ');
    clickSave();
    await flush();

    expect(mockApi.command).toHaveBeenCalledTimes(1);
    expect(mockApi.command).toHaveBeenCalledWith(
      'POST',
      '/api/transactions/tx-v-1/referral',
      { id: 'tx-v-1', referralCode: 'abc123' },
      expect.anything(),
    );
  });

  it('en and zh wording for blank code and referralCode refusal', async () => {
    const { mockApi } = mount(
      vi.fn().mockRejectedValue(
        new ApiError({
          status: 400,
          code: 'validationFailed',
          field: 'referralCode',
          message: 'รหัสแนะนำไม่ถูกต้อง',
        }),
      ),
    );
    await flush();

    // Check blank in EN
    setLang('en');
    await flush();
    type('');
    clickSave();
    await flush();
    expect(alertText()).toBe(en.codeRequired);

    // Check invalid format in EN
    type('bad');
    clickSave();
    await flush();
    expect(alertText()).toBe(en.invalidCode);

    // Check server referralCode refusal in EN
    type('ABCDEF');
    clickSave();
    await flush();
    expect(mockApi.command).toHaveBeenCalledTimes(1);
    expect(alertText()).toBe(en.invalidCode);

    // Check server referralCode refusal in ZH (desk screen falls back to EN)
    setLang('zh');
    await flush();
    expect(alertText()).toBe(en.invalidCode);
  });
});
