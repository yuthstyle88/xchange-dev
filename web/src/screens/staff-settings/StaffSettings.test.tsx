// #6552: /staff/settings settings hub, page entry (`web/pages/staff-settings.html`), and shared settings link helpers.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { en as sharedEn } from '../../shared/i18n/en';
import { th as sharedTh } from '../../shared/i18n/th';
import { zh as sharedZh } from '../../shared/i18n/zh';
import type { SessionStore } from '../../shared/session/session';
import { makeBranchRef, makeMe } from '../../shared/testing/fixtures';

type SettingsLinkItem = {
  group: 'cash' | 'alerts';
  key: 'cashLimits' | 'cashPolicy' | 'alertSettings' | 'rateAlertSettings';
  href: string;
  permission: string;
};

type StaffNavModule = {
  SETTINGS_LINKS?: readonly SettingsLinkItem[];
  visibleSettingsLinks?: (permissions: readonly string[]) => SettingsLinkItem[];
  hasVisibleSettings?: (permissions: readonly string[]) => boolean;
  STAFF_NAV?: Array<{ key: string; href: string; visible?: (permissions: readonly string[]) => boolean }>;
  visibleStaffNav?: (permissions: readonly string[]) => Array<{ key: string; href: string }>;
};

type StaffSettingsProps = {
  session?: SessionStore;
  navigate?: (url: string) => void;
};

type ScreenMod = {
  StaffSettings?: (props?: StaffSettingsProps) => unknown;
};

type I18nMod = {
  th?: Record<string, string>;
  en?: Record<string, string>;
};

const pageEntries = import.meta.glob<string>('../../../pages/staff-settings.html', {
  query: '?raw',
  import: 'default',
  eager: true,
});
const navMods = import.meta.glob<StaffNavModule>('../../shared/ui/staff-nav.ts', { eager: true });
const screenMods = import.meta.glob<ScreenMod>('./StaffSettings.tsx');
const mainMods = import.meta.glob('./main.tsx');
const i18nMods = import.meta.glob<I18nMod>('./i18n.ts', { eager: true });
const sourceEntries = import.meta.glob<string>(
  ['./StaffSettings.tsx', './main.tsx', '../../shared/ui/staff-nav.ts'],
  {
    query: '?raw',
    import: 'default',
    eager: true,
  },
);

async function readHere(relative: string): Promise<string> {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as {
    readFileSync(file: string, encoding: string): string;
  };
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as {
    dirname(p: string): string;
    join(...segments: string[]): string;
  };
  const here = path.dirname(new URL(import.meta.url).pathname);
  return fs.readFileSync(path.join(here, relative), 'utf8');
}

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};

function meWith(permissions: readonly string[], role: 'OWNER' | 'TELLER' | 'CUSTOMER' = 'OWNER') {
  return makeMe({
    id: 'u-1',
    name: 'Somchai',
    roles: [{ role, branchId: role === 'OWNER' || role === 'CUSTOMER' ? null : 'MAIN' }],
    permissions: [...permissions],
    branch: 'MAIN',
    branchName: 'สำนักงานใหญ่',
    branches: [makeBranchRef({ id: 'MAIN', code: 'XC-01', name: 'สำนักงานใหญ่' })],
  });
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let cleanup: Array<() => void> = [];

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

beforeEach(() => {
  try {
    window.localStorage.clear();
  } catch {
    // ignore
  }
  setLang('th');
  window.history.pushState({}, '', '/staff/settings');
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
});

afterEach(() => {
  for (const c of cleanup.reverse()) c();
  cleanup = [];
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  window.history.pushState({}, '', '/');
  setLang('th');
});

async function mountHub(
  meReply:
    | { kind: 'ok'; permissions: readonly string[]; role?: 'OWNER' | 'TELLER' | 'CUSTOMER' }
    | { kind: 'pending' }
    | { kind: 'error'; status: number; code: string; message: string },
  props: StaffSettingsProps = {},
) {
  vi.resetModules();
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const url = String(input);
      if (url.startsWith('/health/ready')) return json(200, READY);
      if (url.startsWith('/api/v3/views/me')) {
        if (meReply.kind === 'pending') {
          return await new Promise<Response>(() => {});
        }
        if (meReply.kind === 'error') {
          return json(meReply.status, { error: { code: meReply.code, message: meReply.message } });
        }
        return json(200, {
          version: 1,
          changed: true,
          data: meWith(meReply.permissions, meReply.role ?? 'OWNER'),
        });
      }
      return json(404, { error: { code: 'notFound', message: 'not found' } });
    }),
  );

  const sessionMod = await import('../../shared/session/session');
  const i18n = await import('../../shared/i18n');
  const modLoader = screenMods['./StaffSettings.tsx'];
  expect(modLoader, 'StaffSettings.tsx exists').toBeTypeOf('function');
  const { StaffSettings } = await modLoader!();
  expect(StaffSettings, 'StaffSettings component exported').toBeTypeOf('function');

  const host = document.createElement('div');
  document.body.appendChild(host);
  const dispose = render(() => (StaffSettings as (p: StaffSettingsProps) => any)(props), host);
  cleanup.push(dispose, () => sessionMod.sharedSession().dispose());
  await settle();
  return { host, session: sessionMod.sharedSession(), i18n };
}

function hubLinks(host: HTMLElement): HTMLAnchorElement[] {
  return Array.from(host.querySelectorAll<HTMLAnchorElement>('[data-settings-group] a'));
}

function hubHrefs(host: HTMLElement): (string | null)[] {
  return hubLinks(host).map((a) => a.getAttribute('href'));
}

function hubGroups(host: HTMLElement): string[] {
  return Array.from(host.querySelectorAll<HTMLElement>('[data-settings-group]')).map(
    (g) => g.getAttribute('data-settings-group') ?? '',
  );
}

describe('web/pages/staff-settings.html (#6552 §3 item 1, §5)', () => {
  it('declares lang="th", xc-path="/staff/settings", xc-gate="signed-in", #app, and /src/screens/staff-settings/main.tsx', () => {
    const raw = pageEntries['../../../pages/staff-settings.html'];
    expect(raw, 'web/pages/staff-settings.html exists').toBeTypeOf('string');
    const doc = new DOMParser().parseFromString(raw!, 'text/html');
    expect(doc.documentElement.getAttribute('lang')).toBe('th');
    expect(doc.title).toContain('ตั้งค่า');
    const metas = (name: string) =>
      Array.from(doc.querySelectorAll(`meta[name="${name}"]`)).map((m) => m.getAttribute('content'));
    expect(metas('xc-path')).toEqual(['/staff/settings']);
    expect(metas('xc-gate')).toEqual(['signed-in']);
    expect(doc.getElementById('app')).not.toBeNull();
    const scripts = Array.from(doc.querySelectorAll('script[src]')).map((s) => ({
      src: s.getAttribute('src'),
      type: s.getAttribute('type'),
    }));
    expect(scripts).toEqual([{ src: '/src/screens/staff-settings/main.tsx', type: 'module' }]);
  });
});

describe('SETTINGS_LINKS, visibleSettingsLinks, and hasVisibleSettings (#6552 §7)', () => {
  const nav = navMods['../../shared/ui/staff-nav.ts'];

  it('defines the 4 destination links in cash and alerts groups with their existing visibility permissions', () => {
    expect(nav?.SETTINGS_LINKS).toEqual([
      { group: 'cash', key: 'cashLimits', href: '/#cash-limits', permission: 'settingsRead' },
      { group: 'cash', key: 'cashPolicy', href: '/#cash-policy', permission: 'settingsRead' },
      { group: 'alerts', key: 'alertSettings', href: '/#alert-settings', permission: 'AlertSettingsSave' },
      { group: 'alerts', key: 'rateAlertSettings', href: '/#rate-alert-settings', permission: 'RateAlertSettingsSave' },
    ]);
  });

  it('visibleSettingsLinks and hasVisibleSettings filter by permission and match StaffNav', () => {
    const hrefs = (perms: readonly string[]) => (nav?.visibleSettingsLinks?.(perms) ?? []).map((l) => l.href);
    expect(hrefs(['settingsRead'])).toEqual(['/#cash-limits', '/#cash-policy']);
    expect(hrefs(['AlertSettingsSave'])).toEqual(['/#alert-settings']);
    expect(hrefs(['RateAlertSettingsSave'])).toEqual(['/#rate-alert-settings']);
    expect(hrefs(['settingsRead', 'AlertSettingsSave', 'RateAlertSettingsSave'])).toEqual([
      '/#cash-limits',
      '/#cash-policy',
      '/#alert-settings',
      '/#rate-alert-settings',
    ]);
    expect(hrefs(['CashLimitsManage', 'CashTransferPolicySet', 'ExchangeTransact'])).toEqual([]);
    expect(nav?.hasVisibleSettings?.([])).toBe(false);
    expect(nav?.hasVisibleSettings?.(['settingsRead'])).toBe(true);
    expect(nav?.hasVisibleSettings?.(['AlertSettingsSave'])).toBe(true);
    expect(nav?.hasVisibleSettings?.(['RateAlertSettingsSave'])).toBe(true);
  });

  it('does not import admin menu/editor modules into the staff hub bundle', () => {
    for (const [path, src] of Object.entries(sourceEntries)) {
      expect(src, path).not.toMatch(/screens\/admin\//);
    }
  });
});

describe('StaffSettings screen behaviour (#6552 §4, §9)', () => {
  it('row 1: settingsRead only -> ตั้งค่า visible; cash-limits and cash-policy links only, even without write permissions', async () => {
    const { host } = await mountHub({ kind: 'ok', permissions: ['SignedIn', 'settingsRead'] });
    expect(hubGroups(host)).toEqual(['cash']);
    expect(hubHrefs(host)).toEqual(['/#cash-limits', '/#cash-policy']);
    const navCurrent = host.querySelector('header .xc-shell-nav a[aria-current="page"]');
    expect(navCurrent?.getAttribute('href')).toBe('/staff/settings');
    expect(navCurrent?.textContent).toContain('ตั้งค่า');
  });

  it('row 2: AlertSettingsSave only -> ตั้งค่า visible; alert-settings only', async () => {
    const { host } = await mountHub({ kind: 'ok', permissions: ['SignedIn', 'AlertSettingsSave'] });
    expect(hubGroups(host)).toEqual(['alerts']);
    expect(hubHrefs(host)).toEqual(['/#alert-settings']);
  });

  it('row 3: RateAlertSettingsSave only -> ตั้งค่า visible; rate-alert-settings only', async () => {
    const { host } = await mountHub({ kind: 'ok', permissions: ['SignedIn', 'RateAlertSettingsSave'] });
    expect(hubGroups(host)).toEqual(['alerts']);
    expect(hubHrefs(host)).toEqual(['/#rate-alert-settings']);
  });

  it('row 4: all three permissions -> fifth nav item, both groups, all four exact hrefs, and one page title', async () => {
    const { host } = await mountHub({
      kind: 'ok',
      permissions: [
        'SignedIn',
        'ExchangeTransact',
        'exchangeBackdate',
        'CashManage',
        'CompatRead',
        'settingsRead',
        'AlertSettingsSave',
        'RateAlertSettingsSave',
      ],
    });
    const navHrefs = Array.from(host.querySelectorAll<HTMLAnchorElement>('header .xc-shell-nav a')).map((a) =>
      a.getAttribute('href'),
    );
    expect(navHrefs).toEqual(['/desk', '/desk/backdated', '/#cash', '/#reports', '/staff/settings']);
    expect(hubGroups(host)).toEqual(['cash', 'alerts']);
    expect(hubHrefs(host)).toEqual([
      '/#cash-limits',
      '/#cash-policy',
      '/#alert-settings',
      '/#rate-alert-settings',
    ]);
    const titles = host.querySelectorAll('h1');
    expect(titles.length, 'exactly one page title').toBe(1);
    expect(titles[0]?.textContent).toBe('ตั้งค่า');
    const headings = Array.from(host.querySelectorAll('[data-settings-group] h2')).map((h) => h.textContent);
    expect(headings).toEqual(['เงินสด', 'การแจ้งเตือน']);
  });

  it('row 5a: permissions pending -> loading state, no settings nav or privileged links', async () => {
    const { host } = await mountHub({ kind: 'pending' });
    expect(hubHrefs(host)).toEqual([]);
    expect(host.querySelector('header .xc-shell-nav')).toBeNull();
    expect(host.querySelector('[data-testid="staff-settings-loading"]')).not.toBeNull();
  });

  it('row 5b: permissions failed -> PermissionsUnavailable recovery UI, no flash of privileged links; retry recovers', async () => {
    let fail = true;
    vi.resetModules();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: string) => {
        const url = String(input);
        if (url.startsWith('/health/ready')) return json(200, READY);
        if (url.startsWith('/api/v3/views/me')) {
          if (fail) return json(500, { error: { code: 'serverError', message: 'db down' } });
          return json(200, { version: 1, changed: true, data: meWith(['SignedIn', 'settingsRead']) });
        }
        return json(404, {});
      }),
    );
    const sessionMod = await import('../../shared/session/session');
    const { StaffSettings } = await screenMods['./StaffSettings.tsx']!();
    const host = document.createElement('div');
    document.body.appendChild(host);
    const dispose = render(() => (StaffSettings as (p: StaffSettingsProps) => any)({}), host);
    cleanup.push(dispose, () => sessionMod.sharedSession().dispose());
    await settle();

    expect(hubHrefs(host)).toEqual([]);
    expect(host.querySelector('header .xc-shell-nav')).toBeNull();
    const alert = host.querySelector('[data-permissions-unavailable]');
    expect(alert).not.toBeNull();
    expect(alert?.getAttribute('role')).toBe('alert');

    fail = false;
    host.querySelector<HTMLButtonElement>('button[data-action="permissions-retry"]')?.click();
    await settle();
    expect(host.querySelector('[data-permissions-unavailable]')).toBeNull();
    expect(hubHrefs(host)).toEqual(['/#cash-limits', '/#cash-policy']);
  });

  it('row 5c: none of the settings permissions -> translated no-access state and no destination links', async () => {
    const { host, i18n } = await mountHub({
      kind: 'ok',
      permissions: ['SignedIn', 'ExchangeTransact', 'CompatRead'],
      role: 'TELLER',
    });
    expect(hubHrefs(host)).toEqual([]);
    expect(hubGroups(host)).toEqual([]);
    const empty = host.querySelector('[data-testid="staff-settings-empty"]');
    expect(empty).not.toBeNull();
    expect(empty?.textContent).toBe(i18nMod().th?.noAccess);
    i18n.setLang('en');
    await settle();
    expect(empty?.textContent).toBe(i18nMod().en?.noAccess);
  });

  it('row 6: permissions revoked while page is open -> links and nav react immediately', async () => {
    const { host, session } = await mountHub({
      kind: 'ok',
      permissions: ['SignedIn', 'ExchangeTransact', 'settingsRead', 'AlertSettingsSave'],
    });
    expect(hubHrefs(host)).toEqual(['/#cash-limits', '/#cash-policy', '/#alert-settings']);
    expect(
      Array.from(host.querySelectorAll<HTMLAnchorElement>('header .xc-shell-nav a')).map((a) => a.getAttribute('href')),
    ).toEqual(['/desk', '/staff/settings']);

    session.updateMe(meWith(['SignedIn', 'ExchangeTransact', 'AlertSettingsSave']));
    await settle();
    expect(hubGroups(host)).toEqual(['alerts']);
    expect(hubHrefs(host)).toEqual(['/#alert-settings']);

    session.updateMe(meWith(['SignedIn', 'ExchangeTransact']));
    await settle();
    expect(hubHrefs(host)).toEqual([]);
    expect(host.querySelector('[data-testid="staff-settings-empty"]')).not.toBeNull();
    expect(
      Array.from(host.querySelectorAll<HTMLAnchorElement>('header .xc-shell-nav a')).map((a) => a.getAttribute('href')),
    ).toEqual(['/desk']);
  });

  it('row 7: signed out or customer-only session -> hub not rendered', async () => {
    const { host: customerHost } = await mountHub({
      kind: 'ok',
      permissions: ['CustomerSelf', 'SignedIn'],
      role: 'CUSTOMER',
    });
    expect(hubHrefs(customerHost)).toEqual([]);
    expect(customerHost.querySelector('header .xc-shell-nav')).toBeNull();
    expect(customerHost.querySelector('[data-testid="staff-settings-empty"]')).not.toBeNull();

    const navigate = vi.fn();
    const { host: signedOutHost } = await mountHub(
      { kind: 'error', status: 401, code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' },
      { navigate },
    );
    expect(hubHrefs(signedOutHost)).toEqual([]);
    expect(signedOutHost.querySelector('header .xc-shell-nav')).toBeNull();
    expect(navigate).toHaveBeenCalledWith('/login?next=%2Fstaff%2Fsettings');
  });

  it('row 8 & 9: direct URL boot via main.tsx, th/en/zh translations, and staff-scoped CSS tokens', async () => {
    vi.resetModules();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: string) => {
        const url = String(input);
        if (url.startsWith('/health/ready')) return json(200, READY);
        if (url.startsWith('/api/v3/views/me')) {
          return json(200, {
            version: 1,
            changed: true,
            data: meWith(['SignedIn', 'settingsRead', 'AlertSettingsSave', 'RateAlertSettingsSave']),
          });
        }
        return json(404, {});
      }),
    );
    const app = document.createElement('div');
    app.id = 'app';
    document.body.appendChild(app);
    const sessionMod = await import('../../shared/session/session');
    const i18n = await import('../../shared/i18n');
    const loadMain = mainMods['./main.tsx'];
    expect(loadMain, 'main.tsx exists').toBeTypeOf('function');
    await loadMain!();
    cleanup.push(() => sessionMod.sharedSession().dispose());
    await settle();

    expect(hubHrefs(app)).toEqual([
      '/#cash-limits',
      '/#cash-policy',
      '/#alert-settings',
      '/#rate-alert-settings',
    ]);
    expect(app.querySelector('h1.xc-shell-title')?.textContent).toBe('ตั้งค่า');

    i18n.setLang('en');
    await settle();
    expect(app.querySelector('h1.xc-shell-title')?.textContent).toBe('Settings');
    const enHeadings = Array.from(app.querySelectorAll('[data-settings-group] h2')).map((h) => h.textContent);
    expect(enHeadings).toEqual(['Cash', 'Alerts']);

    expect((sharedTh as Record<string, string>)['shell.nav.settings']).toBe('ตั้งค่า');
    expect((sharedEn as Record<string, string>)['shell.nav.settings']).toBe('Settings');
    expect((sharedZh as Record<string, string>)['shell.nav.settings']).toBe('设置');

    const css = await readHere('staff-settings.css');
    expect(css).toContain('.xc-staff-settings');
    expect(css).toContain('var(--xc-');
    expect(css).not.toContain('.xc-customer');
  });
});

function i18nMod(): I18nMod {
  return i18nMods['./i18n.ts'] ?? {};
}
