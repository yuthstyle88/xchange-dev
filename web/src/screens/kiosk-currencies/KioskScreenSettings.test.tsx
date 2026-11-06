import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, ApiError, type ViewParams } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import {
  makeKioskCurrencies,
  makeKioskScreenData,
  makeRateFreshnessDto,
  makeViewAnswer,
  type KioskScreenData,
  type KioskScreenSaveInput,
} from '../../shared/testing/fixtures';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';
import { KioskScreenSettings } from './KioskScreenSettings';

const ALL_FIELDS = [
  'quickAmounts',
  'tiles',
  'popularDays',
  'idleSeconds',
  'reviewSeconds',
  'refreshSeconds',
  'defaultPaymentMethod',
];

function unsetScreen(branch = 'MAIN'): KioskScreenData {
  return makeKioskScreenData({
    branch,
    quickAmounts: [100, 500, 1000, 5000],
    tiles: 6,
    popularDays: 30,
    idleSeconds: 120,
    reviewSeconds: 3,
    refreshSeconds: 10,
    defaultPaymentMethod: 'Cash',
    defaults: [...ALL_FIELDS],
    version: null,
  });
}

function createFakeApi(options?: {
  initial?: KioskScreenData;
  onSave?: (input: KioskScreenSaveInput) => Promise<KioskScreenData>;
}) {
  let current = options?.initial ?? unsetScreen();
  const views: Array<{ name: string; q?: ViewParams }> = [];
  const commands: Array<{
    method: string;
    path: string;
    body: KioskScreenSaveInput;
    key: string;
  }> = [];

  const api: Pick<ApiClient, 'view' | 'post' | 'command'> = {
    async view(name: string, q?: ViewParams) {
      views.push({ name, q: q ? { ...q } : undefined });
      if (name === 'kiosk_screen') {
        return makeViewAnswer(current, {
          version: current.version ?? 0,
          changed: true,
        }) as any;
      }
      return makeViewAnswer(
        makeKioskCurrencies({
          currencies: [],
          kbank: null,
          freshness: makeRateFreshnessDto({
            stale: false,
            sourceDate: null,
            publishedAt: null,
          }),
        }),
        { version: 1, changed: true },
      ) as any;
    },
    async post() {
      return { saved: true } as any;
    },
    async command(method: string, path: string, body: unknown, key: string) {
      const input = JSON.parse(JSON.stringify(body)) as KioskScreenSaveInput;
      commands.push({ method, path, body: input, key });
      if (options?.onSave) {
        current = await options.onSave(input);
        return current as any;
      }
      const nextDefaults: string[] = [];
      if (input.quickAmounts === null) nextDefaults.push('quickAmounts');
      if (input.tiles === null) nextDefaults.push('tiles');
      if (input.popularDays === null) nextDefaults.push('popularDays');
      if (input.idleSeconds === null) nextDefaults.push('idleSeconds');
      if (input.reviewSeconds === null) nextDefaults.push('reviewSeconds');
      if (input.refreshSeconds === null) nextDefaults.push('refreshSeconds');
      if (input.defaultPaymentMethod === null) {
        nextDefaults.push('defaultPaymentMethod');
      }
      current = makeKioskScreenData({
        branch: input.branch,
        quickAmounts: input.quickAmounts ?? [100, 500, 1000, 5000],
        tiles: input.tiles ?? 6,
        popularDays: input.popularDays ?? 30,
        idleSeconds: input.idleSeconds ?? 120,
        reviewSeconds: input.reviewSeconds ?? 3,
        refreshSeconds: input.refreshSeconds ?? 10,
        defaultPaymentMethod: input.defaultPaymentMethod ?? 'Cash',
        defaults: nextDefaults,
        version: (input.expectedVersion ?? 0) + 1,
      });
      return current as any;
    },
  };

  return {
    api,
    views,
    commands,
    setCurrent(next: KioskScreenData) {
      current = next;
    },
  };
}

describe('KioskScreenSettings (#6273)', () => {
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

  it('shows "หน้าจอ kiosk" on KioskCurrenciesScreen with "ค่าเริ่มต้น" beside every unset field and TH/EN labels', async () => {
    const fake = createFakeApi();
    dispose = render(
      () => <KioskCurrenciesScreen api={fake.api} branch="MAIN" />,
      host,
    );

    await vi.waitFor(() => {
      expect(
        fake.views.some(
          (v) => v.name === 'kiosk_screen' && v.q?.branch === 'MAIN',
        ),
      ).toBe(true);
    });

    const section = host.querySelector<HTMLElement>(
      'section[data-section="kiosk-screen"]',
    );
    expect(section).not.toBeNull();
    expect(section!.querySelector('h2')?.textContent).toBe('หน้าจอ kiosk');

    for (const field of ALL_FIELDS) {
      const badge = section!.querySelector<HTMLElement>(
        `[data-default-field="${field}"]`,
      );
      expect(badge?.textContent, `${field} default badge`).toBe('ค่าเริ่มต้น');
      const resetBtn = section!.querySelector<HTMLButtonElement>(
        `button[data-reset-field="${field}"]`,
      );
      expect(resetBtn?.textContent, `${field} reset button`).toBe(
        'กลับค่าเริ่มต้น',
      );
    }

    setLang('en');
    await vi.waitFor(() => {
      expect(section!.querySelector('h2')?.textContent).toBe('Kiosk screen');
      expect(
        section!.querySelector('[data-default-field="tiles"]')?.textContent,
      ).toBe('Default');
      expect(
        section!.querySelector('button[data-reset-field="tiles"]')?.textContent,
      ).toBe('Reset to default');
    });
  });

  it('saves edited fields and resets one field back to default ("กลับค่าเริ่มต้น")', async () => {
    const fake = createFakeApi({
      initial: makeKioskScreenData({
        branch: 'MAIN',
        quickAmounts: [100, 200, 500],
        tiles: 8,
        popularDays: 45,
        idleSeconds: 180,
        reviewSeconds: 4,
        refreshSeconds: 15,
        defaultPaymentMethod: 'PromptPay',
        defaults: [],
        version: 2,
      }),
    });

    dispose = render(
      () => <KioskScreenSettings api={fake.api} branch="MAIN" />,
      host,
    );

    const tilesInput = () =>
      host.querySelector<HTMLInputElement>('input[name="tiles"]')!;
    await vi.waitFor(() => {
      expect(tilesInput().value).toBe('8');
      expect(tilesInput().disabled).toBe(false);
    });
    expect(host.querySelector('[data-default-field="tiles"]')).toBeNull();

    // Reset tiles to default
    const resetTiles = host.querySelector<HTMLButtonElement>(
      'button[name="reset_tiles"]',
    )!;
    resetTiles.click();

    expect(tilesInput().value).toBe('6');
    expect(
      host.querySelector('[data-default-field="tiles"]')?.textContent,
    ).toBe('ค่าเริ่มต้น');

    const saveBtn = host.querySelector<HTMLButtonElement>(
      'button[name="save_kiosk_screen"]',
    )!;
    saveBtn.click();

    await vi.waitFor(() => {
      expect(fake.commands).toHaveLength(1);
    });

    expect(fake.commands[0]!.path).toBe('/api/v3/commands/kiosk_screen_save');
    expect(fake.commands[0]!.body).toEqual({
      branch: 'MAIN',
      quickAmounts: [100, 200, 500],
      tiles: null,
      popularDays: 45,
      idleSeconds: 180,
      reviewSeconds: 4,
      refreshSeconds: 15,
      defaultPaymentMethod: 'PromptPay',
      expectedVersion: 2,
    });
  });

  it('validates bounds before sending and reloads on 409 staleVersion', async () => {
    let shouldConflict = true;
    const fake = createFakeApi({
      onSave: async (input) => {
        if (shouldConflict) {
          throw new ApiError({
            status: 409,
            code: 'staleVersion',
            message: 'staleVersion',
          });
        }
        return makeKioskScreenData({
          branch: input.branch,
          quickAmounts: input.quickAmounts ?? [100, 500, 1000, 5000],
          tiles: input.tiles ?? 6,
          popularDays: 30,
          idleSeconds: 120,
          reviewSeconds: 3,
          refreshSeconds: 10,
          defaultPaymentMethod: 'Cash',
          defaults: ['popularDays', 'idleSeconds', 'reviewSeconds', 'refreshSeconds', 'defaultPaymentMethod'],
          version: 5,
        });
      },
    });

    dispose = render(
      () => <KioskScreenSettings api={fake.api} branch="MAIN" />,
      host,
    );

    const chipsInput = () =>
      host.querySelector<HTMLInputElement>('input[name="quickAmounts"]')!;
    const refreshInput = () =>
      host.querySelector<HTMLInputElement>('input[name="refreshSeconds"]')!;
    await vi.waitFor(() => {
      expect(chipsInput().disabled).toBe(false);
    });

    // Out-of-range refreshSeconds (20 > MAX_REFRESH_SECONDS 19) is blocked before command
    refreshInput().value = '20';
    refreshInput().dispatchEvent(new Event('input', { bubbles: true }));
    host
      .querySelector<HTMLButtonElement>('button[name="save_kiosk_screen"]')!
      .click();

    expect(fake.commands).toHaveLength(0);
    expect(
      host.querySelector('[data-error-field="refreshSeconds"]')?.textContent,
    ).toContain('5–19');

    // Reset refreshSeconds and set valid chips, then trigger 409 staleVersion -> reloads view
    host
      .querySelector<HTMLButtonElement>('button[name="reset_refreshSeconds"]')!
      .click();
    chipsInput().value = '100, 200, 500';
    chipsInput().dispatchEvent(new Event('input', { bubbles: true }));

    fake.setCurrent({
      ...unsetScreen('MAIN'),
      quickAmounts: [200, 400],
      defaults: ALL_FIELDS.filter((f) => f !== 'quickAmounts'),
      version: 4,
    });

    host
      .querySelector<HTMLButtonElement>('button[name="save_kiosk_screen"]')!
      .click();

    await vi.waitFor(() => {
      expect(fake.commands).toHaveLength(1);
      expect(fake.views.filter((v) => v.name === 'kiosk_screen').length).toBe(2);
      expect(chipsInput().value).toBe('200, 400');
    });

    shouldConflict = false;
  });
});
