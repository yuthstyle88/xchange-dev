// QW-6 (#4373, §5r F15, §4 D14): admin kiosk setting result_seconds on /kiosk-currencies.html
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewParams } from '../../shared/api/client';
import {
  makeKbankReference,
  makeKioskCurrencies,
  makeRateFreshnessDto,
  makeViewAnswer,
} from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

interface FakeClient {
  api: Pick<ApiClient, 'view' | 'post'>;
  views: Array<{ name: string; q?: ViewParams }>;
  posts: Array<{ path: string; body: unknown }>;
}

function createFakeClient(
  onPost: (path: string, body: unknown) => Promise<unknown> = async () => ({
    resultSeconds: 10,
    version: 0,
  }),
): FakeClient {
  const views: FakeClient['views'] = [];
  const posts: FakeClient['posts'] = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view(name: string, q?: ViewParams) {
      views.push({ name, q: q ? { ...q } : undefined });
      const answer = makeViewAnswer(
        makeKioskCurrencies({
          currencies: [],
          kbank: makeKbankReference({ source: 'kbank', publishedAt: '2026-10-06T05:00:00Z', round: 1 }),
          freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
        }),
        { version: 1, changed: true },
      );
      return Promise.resolve(answer as any);
    },
    post<B, R>(path: string, body: B): Promise<R> {
      posts.push({ path, body: JSON.parse(JSON.stringify(body)) as unknown });
      return onPost(path, body) as Promise<R>;
    },
  };
  return { api, views, posts };
}

describe('KioskCurrenciesScreen result_seconds setting (QW-6 #4373)', () => {
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
    document.body.innerHTML = '';
    setLang('th');
  });

  const getResultInput = () =>
    host.querySelector<HTMLInputElement>('input#result_seconds, input[name="result_seconds"]')!;

  const getSaveButton = () =>
    host.querySelector<HTMLButtonElement>('button[name="save_result_seconds"]')!;

  const getReadButton = () =>
    host.querySelector<HTMLButtonElement>('button[name="load_result_seconds"]')!;

  const getFieldError = () =>
    host.querySelector<HTMLElement>('[role="alert"], [data-error="validationFailed"]');

  it('the read: reads settings through POST /api/kiosk and populates result_seconds', async () => {
    const fake = createFakeClient(async (_path, body) => {
      const b = body as { action: string };
      if (b.action === 'settings') {
        return { resultSeconds: 15, version: 1 };
      }
      return { saved: true };
    });

    dispose = render(() => <KioskCurrenciesScreen api={fake.api} branch="B9" />, host);

    // Initial input exists
    const input = getResultInput();
    expect(input).not.toBeNull();

    // Trigger read through settings (via button or focus)
    const readBtn = getReadButton();
    readBtn.click();

    await vi.waitFor(() => {
      expect(fake.posts).toContainEqual({
        path: '/api/kiosk',
        body: { action: 'settings', payload: { branchId: 'B9' } },
      });
    });

    await vi.waitFor(() => {
      expect(input.value).toBe('15');
    });
  });

  it('the save body: saves configureSettings through POST /api/kiosk with branchId and resultSeconds', async () => {
    const fake = createFakeClient(async (_path, body) => {
      const b = body as { action: string; payload?: { resultSeconds?: number } };
      if (b.action === 'configureSettings') {
        return { resultSeconds: b.payload?.resultSeconds ?? 25, version: 2 };
      }
      return { resultSeconds: 20, version: 0 };
    });

    dispose = render(() => <KioskCurrenciesScreen api={fake.api} branch="B9" />, host);

    const input = getResultInput();
    // R4-1: the setting is saved only once it has been read for this branch.
    getReadButton().click();
    await vi.waitFor(() => {
      expect(input.value).toBe('20');
    });

    input.value = '25';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const saveBtn = getSaveButton();
    saveBtn.click();

    await vi.waitFor(() => {
      expect(fake.posts).toContainEqual({
        path: '/api/kiosk',
        body: {
          action: 'configureSettings',
          payload: { branchId: 'B9', resultSeconds: 25 },
        },
      });
    });

    expect(input.value).toBe('25');
  });

  it('the refusal: 400 validationFailed shows error beside the field and keeps the input', async () => {
    const fake = createFakeClient(async (_path, body) => {
      const b = body as { action: string };
      if (b.action === 'configureSettings') {
        throw new ApiError({
          status: 400,
          code: 'validationFailed',
          field: 'resultSeconds',
          message: 'validationFailed',
        });
      }
      return { resultSeconds: 20, version: 0 };
    });

    dispose = render(() => <KioskCurrenciesScreen api={fake.api} branch="B9" />, host);

    const input = getResultInput();
    // R4-1: the setting is saved only once it has been read for this branch.
    getReadButton().click();
    await vi.waitFor(() => {
      expect(input.value).toBe('20');
    });

    input.value = '999';
    input.dispatchEvent(new Event('input', { bubbles: true }));

    const saveBtn = getSaveButton();
    saveBtn.click();

    await vi.waitFor(() => {
      const err = getFieldError();
      expect(err, 'error must show beside the field').not.toBeNull();
      expect(err!.textContent).toContain('3–120');
    });

    // Input keeps the typed input value upon refusal (not reset), and out-of-range is blocked before POST
    expect(input.value).toBe('999');
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(fake.posts.filter((p) => (p.body as { action?: string })?.action === 'configureSettings')).toEqual([]);
  });
});
