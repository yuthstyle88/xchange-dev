import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import { ReferralCodeField } from './ReferralCodeField';

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
});

describe('ReferralCodeField permission gate (DK-R2 #1355)', () => {
  it('with ReferralAttach, the field shows', async () => {
    const mockApi = {
      view: vi.fn(async (name: string) => {
        if (name === 'me') {
          return { data: { permissions: ['ReferralAttach'] } };
        }
        throw new Error(`unexpected view: ${name}`);
      }),
      command: vi.fn().mockResolvedValue({ captureId: 'cap-1', status: 'ACTIVE' }),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-100" api={mockApi} />, host);
    await flush();

    expect(mockApi.view).toHaveBeenCalledWith('me');
    expect(host.querySelector('.desk-referral-field')).not.toBeNull();
    expect(host.querySelector('.desk-referral-input')).not.toBeNull();
    expect(host.querySelector('.desk-referral-btn')).not.toBeNull();

    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'TELLER88';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('.desk-referral-btn')!.click();
    await flush();

    expect(mockApi.command).toHaveBeenCalledWith(
      'POST',
      '/api/transactions/tx-100/referral',
      { id: 'tx-100', referralCode: 'TELLER88' },
      expect.any(String),
    );
  });

  it('owner (EXECUTIVE+ADMIN) with ReferralAttach on me (while staff_me answers 401) sends the referral code', async () => {
    const mockApi = {
      view: vi.fn(async (name: string) => {
        if (name === 'staff_me') {
          throw new ApiError({ status: 401, code: 'unauthorized', message: 'unauthorized' });
        }
        if (name === 'me') {
          return {
            data: {
              roles: [
                { role: 'EXECUTIVE', branchId: null },
                { role: 'ADMIN', branchId: null },
              ],
              permissions: ['ReferralAttach'],
            },
          };
        }
        throw new Error(`unexpected view: ${name}`);
      }),
      command: vi.fn().mockResolvedValue({ captureId: 'cap-owner', status: 'ACTIVE' }),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-owner-1" api={mockApi} />, host);
    await flush();

    expect(mockApi.view).toHaveBeenCalledWith('me');
    expect(mockApi.view).not.toHaveBeenCalledWith('staff_me');
    expect(host.querySelector('.desk-referral-field')).not.toBeNull();

    const input = host.querySelector<HTMLInputElement>('.desk-referral-input')!;
    input.value = 'OWNER99';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('.desk-referral-btn')!.click();
    await flush();

    expect(mockApi.command).toHaveBeenCalledTimes(1);
    expect(mockApi.command).toHaveBeenCalledWith(
      'POST',
      '/api/transactions/tx-owner-1/referral',
      { id: 'tx-owner-1', referralCode: 'OWNER99' },
      expect.any(String),
    );
  });

  it('without it, nothing renders and no attachReferral call is possible', async () => {
    const mockApi = {
      view: vi.fn(async (name: string) => {
        if (name === 'me') {
          return { data: { permissions: ['BranchDayOperate'] } };
        }
        throw new Error(`unexpected view: ${name}`);
      }),
      command: vi.fn(),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-101" api={mockApi} />, host);
    await flush();

    expect(mockApi.view).toHaveBeenCalledWith('me');
    expect(host.querySelector('.desk-referral-field')).toBeNull();
    expect(host.querySelector('.desk-referral-input')).toBeNull();
    expect(host.querySelector('.desk-referral-btn')).toBeNull();
    expect(host.innerHTML).toBe('');
    expect(mockApi.command).not.toHaveBeenCalled();

    // Also verify when permissions list is empty
    dispose();
    host.innerHTML = '';

    const mockApiEmpty = {
      view: vi.fn(async (name: string) => {
        if (name === 'me') {
          return { data: { permissions: [] } };
        }
        throw new Error(`unexpected view: ${name}`);
      }),
      command: vi.fn(),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-102" api={mockApiEmpty} />, host);
    await flush();

    expect(host.querySelector('.desk-referral-field')).toBeNull();
    expect(host.innerHTML).toBe('');
    expect(mockApiEmpty.command).not.toHaveBeenCalled();
  });

  // Retired by G-ME (docs/design/web-silent-failures.md §5, the #1719 standard): "when me fails, the field is hidden"
  // was the silent hide. A failed `me` read is now worded and the field is shown disabled with the reason; see
  // ReferralCodeField.me-error.test.tsx.
});
