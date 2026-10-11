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
        if (name === 'staff_me') {
          return { data: { permissions: ['ReferralAttach'] } };
        }
        throw new Error(`unexpected view: ${name}`);
      }),
      command: vi.fn(),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-100" api={mockApi} />, host);
    await flush();

    expect(mockApi.view).toHaveBeenCalledWith('staff_me');
    expect(host.querySelector('.desk-referral-field')).not.toBeNull();
    expect(host.querySelector('.desk-referral-input')).not.toBeNull();
    expect(host.querySelector('.desk-referral-btn')).not.toBeNull();
  });

  it('without it, nothing renders and no attachReferral call is possible', async () => {
    const mockApi = {
      view: vi.fn(async (name: string) => {
        if (name === 'staff_me') {
          return { data: { permissions: ['BranchDayOperate'] } };
        }
        throw new Error(`unexpected view: ${name}`);
      }),
      command: vi.fn(),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-101" api={mockApi} />, host);
    await flush();

    expect(mockApi.view).toHaveBeenCalledWith('staff_me');
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
        if (name === 'staff_me') {
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

  it('when staff_me fails, the field is hidden', async () => {
    // Network TypeError
    const mockApiNetworkErr = {
      view: vi.fn(async () => {
        throw new TypeError('Failed to fetch');
      }),
      command: vi.fn(),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-103" api={mockApiNetworkErr} />, host);
    await flush();

    expect(host.querySelector('.desk-referral-field')).toBeNull();
    expect(host.innerHTML).toBe('');
    expect(mockApiNetworkErr.command).not.toHaveBeenCalled();

    // 401 ApiError
    dispose();
    host.innerHTML = '';

    const mockApi401 = {
      view: vi.fn(async () => {
        throw new ApiError({ status: 401, code: 'unauthorized', message: 'unauthorized' });
      }),
      command: vi.fn(),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-104" api={mockApi401} />, host);
    await flush();

    expect(host.querySelector('.desk-referral-field')).toBeNull();
    expect(host.innerHTML).toBe('');
    expect(mockApi401.command).not.toHaveBeenCalled();

    // 500 ApiError
    dispose();
    host.innerHTML = '';

    const mockApi500 = {
      view: vi.fn(async () => {
        throw new ApiError({ status: 500, code: 'internalError', message: 'internal error' });
      }),
      command: vi.fn(),
    } as unknown as ApiClient;

    dispose = render(() => <ReferralCodeField transactionId="tx-105" api={mockApi500} />, host);
    await flush();

    expect(host.querySelector('.desk-referral-field')).toBeNull();
    expect(host.innerHTML).toBe('');
    expect(mockApi500.command).not.toHaveBeenCalled();
  });
});
