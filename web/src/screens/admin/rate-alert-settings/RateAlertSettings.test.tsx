// AL-6 (#1011): admin rate-alert settings screen specs (design `docs/design/blueprint/rate-alerts.md` §3, §4.4, §8, §9, §10).
// Tests cover the edge cases in ticket #1011 §9:
// - loading, empty (no row), error, retry (each state renders; empty state says alerts are closed until saved)
// - save (calls the command once with cooldown in seconds)
// - a session without RateAlertSettingsSave (menu.needs is 'RateAlertSettingsSave', pinned in menus.test.ts)
// - lifetime left blank (defaultLifetimeDays is omitted, no expiry, never sends null)
// - 0 or negative values (refused in the form before sending)

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import {
  makeRateAlertSettingsData,
  makeSaveRateAlertSettingsInput,
  type RateAlertSettingsData,
  type SaveRateAlertSettingsInput,
} from '../../../shared/testing/fixtures';
import type { AdminMenuProps } from '../menus';
import { menu } from './menu';
import { RateAlertSettings } from './RateAlertSettings';
import * as table from './i18n';

interface SentRequest {
  method: string;
  path: string;
  body: unknown;
  headers: Record<string, string>;
}

function dummyProps(overrides: Partial<AdminMenuProps> = {}): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-12', to: '2026-10-12' });
  const [shown] = createSignal(true);
  return {
    branch,
    period,
    shown,
    ...overrides,
  };
}

describe('AL-6: RateAlertSettings (§9 specs)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];
  let viewResponse: { status: number; code?: string; data?: RateAlertSettingsData | null; error?: string } = {
    status: 200,
    data: null,
  };
  let saveResponse: { status: number; code?: string; data?: RateAlertSettingsData; error?: string } = {
    status: 200,
    data: makeRateAlertSettingsData({
      defaultCooldownSeconds: 86400,
      maxActivePerCustomer: 10,
      defaultLifetimeDays: 30,
      observePolicy: 'ALL_FRESH',
    }),
  };

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];
    viewResponse = { status: 200, data: null };
    saveResponse = {
      status: 200,
      data: makeRateAlertSettingsData({
        defaultCooldownSeconds: 86400,
        maxActivePerCustomer: 10,
        defaultLifetimeDays: 30,
        observePolicy: 'ALL_FRESH',
      }),
    };

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k] = v;
        });
      }
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({ method, path: u.pathname, body, headers });

      if (u.pathname === '/api/v3/views/rate_alert_settings') {
        if (viewResponse.status !== 200) {
          return new Response(
            JSON.stringify({ error: { code: viewResponse.code ?? 'serverError', message: viewResponse.error ?? 'View failed' } }),
            { status: viewResponse.status, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: viewResponse.data }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (u.pathname === '/api/rate-alert-settings' && method === 'PUT') {
        if (saveResponse.status !== 200) {
          return new Response(
            JSON.stringify({ error: { code: saveResponse.code ?? 'roleNotAllowed', message: saveResponse.error ?? 'Save failed' } }),
            { status: saveResponse.status, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify(saveResponse.data),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
    });
  });

  afterEach(() => {
    if (dispose) {
      dispose();
      dispose = null;
    }
    container.remove();
    vi.restoreAllMocks();
  });

  function mount(props: AdminMenuProps = dummyProps()) {
    dispose = render(() => <RateAlertSettings {...props} />, container);
  }

  // 1. Menu and visibility
  it('menu has id rate-alert-settings and needs RateAlertSettingsSave (visible to EXECUTIVE only)', () => {
    expect(menu.id).toBe('rate-alert-settings');
    expect(menu.needs).toBe('RateAlertSettingsSave');
    expect(menu.group).toBe('tools');
    expect(menu.dated).toBe(false);
    expect(typeof menu.label).toBe('function');
    expect(typeof menu.heading.title).toBe('function');
    expect(menu.Component).toBe(RateAlertSettings);
  });

  // 2. Loading state, empty state (no row), and note that alerts are closed until saved (pinned on .rate-alert-banner in TH and EN)
  it('renders loading state, then empty state with note that alerts are closed until saved in TH and EN', async () => {
    // 2a. Check in Thai
    viewResponse = { status: 200, data: null };
    mount();

    // Verify view was requested
    expect(sentRequests.some((r) => r.path === '/api/v3/views/rate_alert_settings')).toBe(true);

    // Wait for empty banner to resolve
    await vi.waitFor(() => {
      expect(container.querySelector('.rate-alert-banner')).not.toBeNull();
    });

    const bannerTh = container.querySelector('.rate-alert-banner');
    expect(bannerTh).not.toBeNull();
    expect(bannerTh?.textContent).toContain(table.th.emptyNote);
    expect(bannerTh?.textContent).toContain(table.th.emptyTitle);

    // Verify observePolicy starts unselected on empty load (not pre-selecting ALL_FRESH)
    const selectedPolicy = container.querySelector('input[name="observePolicy"]:checked');
    expect(selectedPolicy).toBeNull();

    // Verify form inputs are empty initially
    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const maxActiveInput = container.querySelector('input[name="maxActivePerCustomer"]') as HTMLInputElement;
    const lifetimeInput = container.querySelector('input[name="defaultLifetimeDays"]') as HTMLInputElement;

    expect(cooldownInput.value).toBe('');
    expect(maxActiveInput.value).toBe('');
    expect(lifetimeInput.value).toBe('');

    // Suggested values are shown as suggestions / placeholders
    expect(container.textContent).toContain(table.th.suggested);
    expect(container.textContent).toContain('24');
    expect(container.textContent).toContain('10');
    expect(container.textContent).toContain('30');

    // Nothing was saved without user pressing save
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);

    // 2b. Check in English
    if (dispose) {
      dispose();
      dispose = null;
    }
    container.innerHTML = '';
    setLang('en');
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('.rate-alert-banner')).not.toBeNull();
    });

    const bannerEn = container.querySelector('.rate-alert-banner');
    expect(bannerEn).not.toBeNull();
    expect(bannerEn?.textContent).toContain(table.en.emptyNote);
    expect(bannerEn?.textContent).toContain(table.en.emptyTitle);
  });

  // 3. Error and retry state
  it('renders error state when view fails using errorText in words, and re-fetches on retry', async () => {
    viewResponse = { status: 500, code: 'serverError', error: 'Database connection failed' };
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-state="error"]')).not.toBeNull();
    });

    // In Thai, errorText('serverError', ...) yields the Thai phrase 'เซิร์ฟเวอร์ขัดข้อง'
    expect(container.textContent).toContain(errorText('serverError', 'Database connection failed'));
    const retryBtn = container.querySelector('button[data-action="retry"]') as HTMLButtonElement;
    expect(retryBtn).not.toBeNull();

    // Prepare success response for retry
    viewResponse = {
      status: 200,
      data: makeRateAlertSettingsData({
        defaultCooldownSeconds: 86400,
        maxActivePerCustomer: 10,
        defaultLifetimeDays: 30,
        observePolicy: 'ALL_FRESH',
      }),
    };

    sentRequests = [];
    retryBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-state="loaded"]')).not.toBeNull();
    });

    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    expect(cooldownInput.value).toBe('24');
  });

  // 4. Save: converts cooldown hours to seconds and sends command once
  it('calls rateAlertSettingsSave once with cooldown in seconds on save', async () => {
    viewResponse = { status: 200, data: null };
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });

    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const maxActiveInput = container.querySelector('input[name="maxActivePerCustomer"]') as HTMLInputElement;
    const lifetimeInput = container.querySelector('input[name="defaultLifetimeDays"]') as HTMLInputElement;
    const allFreshRadio = container.querySelector('input[value="ALL_FRESH"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;

    cooldownInput.value = '12';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    maxActiveInput.value = '5';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));
    lifetimeInput.value = '15';
    lifetimeInput.dispatchEvent(new Event('input', { bubbles: true }));
    allFreshRadio.click();

    saveBtn.click();

    await vi.waitFor(() => {
      const put = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT');
      expect(put).toBeDefined();
    });

    const putRequest = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT')!;
    const body = makeSaveRateAlertSettingsInput(putRequest.body as Partial<SaveRateAlertSettingsInput>);
    expect(body.defaultCooldownSeconds).toBe(12 * 3600); // 43200 seconds
    expect(body.maxActivePerCustomer).toBe(5);
    expect(body.defaultLifetimeDays).toBe(15);
    expect(body.observePolicy).toBe('ALL_FRESH');
    expect(putRequest.headers['idempotency-key']).toBeTruthy();
  });

  // 5. Blank lifetime: defaultLifetimeDays is omitted, never sent as null
  it('omits defaultLifetimeDays when left blank (never sends null)', async () => {
    viewResponse = { status: 200, data: null };
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });

    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const maxActiveInput = container.querySelector('input[name="maxActivePerCustomer"]') as HTMLInputElement;
    const lifetimeInput = container.querySelector('input[name="defaultLifetimeDays"]') as HTMLInputElement;
    const allFreshRadio = container.querySelector('input[value="ALL_FRESH"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;

    cooldownInput.value = '24';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    maxActiveInput.value = '10';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));
    lifetimeInput.value = ''; // left blank
    lifetimeInput.dispatchEvent(new Event('input', { bubbles: true }));
    allFreshRadio.click();

    saveBtn.click();

    await vi.waitFor(() => {
      const put = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT');
      expect(put).toBeDefined();
    });

    const putRequest = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT')!;
    const body = putRequest.body as Record<string, unknown>;
    expect(body.defaultCooldownSeconds).toBe(86400);
    expect(body.maxActivePerCustomer).toBe(10);
    expect(body.defaultLifetimeDays).toBeUndefined();
    expect('defaultLifetimeDays' in body).toBe(false);
  });

  // 6. 0 or negative values: refused in the form before sending (pins 0 and negative for all 3 fields)
  it('refuses 0 or negative values in the form before sending for all 3 fields', async () => {
    viewResponse = { status: 200, data: null };
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });

    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const maxActiveInput = container.querySelector('input[name="maxActivePerCustomer"]') as HTMLInputElement;
    const lifetimeInput = container.querySelector('input[name="defaultLifetimeDays"]') as HTMLInputElement;
    const allFreshRadio = container.querySelector('input[value="ALL_FRESH"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;
    allFreshRadio.click();

    // 6a. Cooldown: test 0
    cooldownInput.value = '0';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    maxActiveInput.value = '10';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="cooldown"]')).not.toBeNull();
    });
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);

    // 6b. Cooldown: test negative (-5)
    cooldownInput.value = '-5';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="cooldown"]')).not.toBeNull();
    });
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);

    // Fix cooldown to valid value
    cooldownInput.value = '24';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));

    // 6c. Max active: test 0
    maxActiveInput.value = '0';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="maxActive"]')).not.toBeNull();
    });
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);

    // 6d. Max active: test negative (-3)
    maxActiveInput.value = '-3';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="maxActive"]')).not.toBeNull();
    });
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);

    // Fix max active to valid value
    maxActiveInput.value = '10';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));

    // 6e. Lifetime: test 0 (kills Mutant F)
    lifetimeInput.value = '0';
    lifetimeInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="lifetime"]')).not.toBeNull();
    });
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);

    // 6f. Lifetime: test negative (-5)
    lifetimeInput.value = '-5';
    lifetimeInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="lifetime"]')).not.toBeNull();
    });
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);
  });

  // 7. Observe policy required: empty form has no policy selected; save refused until chosen or suggested
  it('refuses save when observePolicy is unselected on empty load, until chosen or suggested values used', async () => {
    viewResponse = { status: 200, data: null };
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });

    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const maxActiveInput = container.querySelector('input[name="maxActivePerCustomer"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;

    cooldownInput.value = '24';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    maxActiveInput.value = '10';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));

    // No policy selected
    expect(container.querySelector('input[name="observePolicy"]:checked')).toBeNull();

    saveBtn.click();

    // Refused before sending, words shown
    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="observePolicy"]')).not.toBeNull();
    });
    expect(container.querySelector('[data-error="observePolicy"]')?.textContent).toContain(table.th.errorPolicyRequired);
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);

    // Click "Use suggested"
    const suggestBtn = container.querySelector('.rate-alert-suggest-btn') as HTMLButtonElement;
    expect(suggestBtn).not.toBeNull();
    suggestBtn.click();

    // ALL_FRESH should now be selected and policy error cleared
    const allFreshRadio = container.querySelector('input[value="ALL_FRESH"]') as HTMLInputElement;
    expect(allFreshRadio.checked).toBe(true);
    expect(container.querySelector('[data-error="observePolicy"]')).toBeNull();
    expect(cooldownInput.value).toBe('24');
    expect(maxActiveInput.value).toBe('10');

    // Now save succeeds
    saveBtn.click();

    await vi.waitFor(() => {
      const put = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT');
      expect(put).toBeDefined();
    });
  });

  // 8. Refusal on save displays errorText in words and preserves user inputs
  it('displays errorText in words near save button on command refusal and preserves user inputs', async () => {
    viewResponse = { status: 200, data: null };
    saveResponse = {
      status: 403,
      code: 'roleNotAllowed',
      error: 'EXECUTIVE role required',
    };

    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });

    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const maxActiveInput = container.querySelector('input[name="maxActivePerCustomer"]') as HTMLInputElement;
    const lifetimeInput = container.querySelector('input[name="defaultLifetimeDays"]') as HTMLInputElement;
    const autoFreshRadio = container.querySelector('input[value="AUTO_FRESH"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;

    cooldownInput.value = '18';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    maxActiveInput.value = '8';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));
    lifetimeInput.value = '45';
    lifetimeInput.dispatchEvent(new Event('input', { bubbles: true }));
    autoFreshRadio.click();

    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="save"]')).not.toBeNull();
    });

    // The load error box must NOT replace the form
    expect(container.querySelector('[data-state="error"]')).toBeNull();
    expect(container.querySelector('form')).not.toBeNull();

    // The save error is shown near save button and words the refusal through errorText
    const saveErrorEl = container.querySelector('[data-error="save"]');
    expect(saveErrorEl).not.toBeNull();
    expect(saveErrorEl?.textContent).toContain(errorText('roleNotAllowed', 'EXECUTIVE role required'));

    // All input values typed by EXECUTIVE are preserved!
    expect(cooldownInput.value).toBe('18');
    expect(maxActiveInput.value).toBe('8');
    expect(lifetimeInput.value).toBe('45');
    expect(autoFreshRadio.checked).toBe(true);
  });

  // 9. Observe policy options with explanations in words
  it('explains ALL_FRESH, ALL, AUTO_FRESH in words and allows selection', async () => {
    viewResponse = { status: 200, data: null };
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });

    const text = container.textContent ?? '';
    // Must explain all 3 policies
    expect(text).toContain('ALL_FRESH');
    expect(text).toContain('ALL');
    expect(text).toContain('AUTO_FRESH');

    // Select AUTO_FRESH
    const autoFreshRadio = container.querySelector('input[value="AUTO_FRESH"]') as HTMLInputElement;
    expect(autoFreshRadio).not.toBeNull();
    autoFreshRadio.click();

    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const maxActiveInput = container.querySelector('input[name="maxActivePerCustomer"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;

    cooldownInput.value = '24';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    maxActiveInput.value = '10';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();

    await vi.waitFor(() => {
      const put = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT');
      expect(put).toBeDefined();
    });

    const putRequest = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT')!;
    const body = makeSaveRateAlertSettingsInput(putRequest.body as Partial<SaveRateAlertSettingsInput>);
    expect(body.observePolicy).toBe('AUTO_FRESH');
  });

  // 10. Texts in both TH and EN
  it('renders correctly in both Thai and English', async () => {
    viewResponse = {
      status: 200,
      data: makeRateAlertSettingsData({
        defaultCooldownSeconds: 86400,
        maxActivePerCustomer: 10,
        defaultLifetimeDays: 30,
        observePolicy: 'ALL_FRESH',
      }),
    };

    setLang('en');
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-state="loaded"]')).not.toBeNull();
    });

    expect(container.textContent).toMatch(/Rate Alert Settings|Cooldown|Max Active/i);
    expect(table.th.title).toBeTruthy();
    expect(table.en.title).toBeTruthy();
    expect(table.th.errorPolicyRequired).toBeTruthy();
    expect(table.en.errorPolicyRequired).toBeTruthy();
  });

  // 11. Cooldown decimals / unrounded: 5400 seconds shows 1.5 hours, and re-saving unchanged sends 5400
  it('does not round stored cooldown: 5400 s loads as 1.5 h and re-saving sends 5400', async () => {
    viewResponse = {
      status: 200,
      data: makeRateAlertSettingsData({
        defaultCooldownSeconds: 5400,
        maxActivePerCustomer: 10,
        defaultLifetimeDays: 30,
        observePolicy: 'ALL_FRESH',
      }),
    };
    saveResponse = {
      status: 200,
      data: makeRateAlertSettingsData({
        defaultCooldownSeconds: 5400,
        maxActivePerCustomer: 10,
        defaultLifetimeDays: 30,
        observePolicy: 'ALL_FRESH',
      }),
    };
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-state="loaded"]')).not.toBeNull();
    });

    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    expect(cooldownInput.value).toBe('1.5');

    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;
    saveBtn.click();

    await vi.waitFor(() => {
      const put = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT');
      expect(put).toBeDefined();
    });

    const putRequest = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT')!;
    const body = makeSaveRateAlertSettingsInput(putRequest.body as Partial<SaveRateAlertSettingsInput>);
    expect(body.defaultCooldownSeconds).toBe(5400);
    expect(cooldownInput.value).toBe('1.5');
  });
});
