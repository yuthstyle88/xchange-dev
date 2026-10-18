import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { LoginHistory } from './LoginHistory';

const disposers: Array<() => void> = [];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function refusal(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (err: unknown) => void;
}

function deferred<T = Response>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

let calls: Array<{ url: string; method: string }>;
let handler: ((url: string) => Promise<Response> | Response) | null = null;

async function serve(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  const method = (init?.method ?? 'GET').toUpperCase();
  calls.push({ url, method });

  if (handler) {
    return handler(url);
  }
  return json({ error: { code: 'notFound', message: 'not found' } }, 404);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function mount(shown: () => boolean, branch: () => string): HTMLElement {
  const container = document.createElement('div');
  document.body.appendChild(container);
  const dispose = render(() => <LoginHistory shown={shown} branch={branch} />, container);
  disposers.push(() => {
    dispose();
    container.remove();
  });
  return container;
}

describe('LoginHistory', () => {
  beforeEach(() => {
    setLang('th');
    calls = [];
    handler = null;
    vi.stubGlobal('fetch', serve);
  });

  afterEach(() => {
    for (const d of disposers.splice(0)) d();
    vi.unstubAllGlobals();
  });

  it('shown() false: makes no request to admin_login_history', async () => {
    const [shown] = createSignal(false);
    const [branch] = createSignal('XC-B01');
    mount(shown, branch);
    await flush();

    expect(calls.filter((c) => c.url.includes('admin_login_history'))).toHaveLength(0);
  });

  it('shown() becomes true: makes one GET request with branch and limit 100, and renders rows in server order', async () => {
    const rows = [
      {
        id: '1',
        at: '2026-10-06T02:30:00Z',
        userId: 'u-1',
        userName: 'Somchai First',
        ip: '192.168.1.1',
        result: 'OK',
      },
      {
        id: '2',
        at: '2026-10-06T03:30:00Z',
        userId: 'u-2',
        userName: 'Somsri Second',
        ip: '192.168.1.2',
        result: 'BAD_PASSWORD',
      },
    ];
    handler = () => json({ version: 1, changed: true, data: rows });

    const [shown, setShown] = createSignal(false);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();
    expect(calls).toHaveLength(0);

    setShown(true);
    await flush();

    const getCalls = calls.filter((call) => call.url.includes('admin_login_history'));
    expect(getCalls).toHaveLength(1);
    expect(getCalls[0].url).toContain('branch=XC-B01');
    expect(getCalls[0].url).toContain('limit=100');

    const table = c.querySelector('table');
    expect(table).not.toBeNull();
    const trs = c.querySelectorAll('tbody tr');
    expect(trs).toHaveLength(2);
    expect(trs[0].textContent).toContain('Somchai First');
    expect(trs[1].textContent).toContain('Somsri Second');
  });

  it('the branch changes while shown: fetches with the new branch and replaces old rows', async () => {
    const rowsB01 = [
      {
        id: '1',
        at: '2026-10-06T02:30:00Z',
        userId: 'u-1',
        userName: 'User B01',
        ip: '192.168.1.1',
        result: 'OK',
      },
    ];
    const rowsB02 = [
      {
        id: '2',
        at: '2026-10-06T04:30:00Z',
        userId: 'u-2',
        userName: 'User B02',
        ip: '192.168.1.2',
        result: 'OK',
      },
    ];

    handler = (url) => {
      if (url.includes('branch=XC-B01')) {
        return json({ version: 1, changed: true, data: rowsB01 });
      }
      if (url.includes('branch=XC-B02')) {
        return json({ version: 1, changed: true, data: rowsB02 });
      }
      return json([]);
    };

    const [shown] = createSignal(true);
    const [branch, setBranch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    expect(c.textContent).toContain('User B01');
    expect(c.textContent).not.toContain('User B02');

    setBranch('XC-B02');
    await flush();

    const getCalls = calls.filter((call) => call.url.includes('admin_login_history'));
    expect(getCalls).toHaveLength(2);
    expect(getCalls[1].url).toContain('branch=XC-B02');
    expect(c.textContent).toContain('User B02');
    expect(c.textContent).not.toContain('User B01');
  });

  it("an older branch's answer arriving after a newer one is ignored", async () => {
    const defB01 = deferred();
    const defB02 = deferred();

    handler = (url) => {
      if (url.includes('branch=XC-B01')) return defB01.promise;
      if (url.includes('branch=XC-B02')) return defB02.promise;
      return json([]);
    };

    const [shown] = createSignal(true);
    const [branch, setBranch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    setBranch('XC-B02');
    await flush();

    // Resolve newer branch B02 first
    defB02.resolve(
      json({
        version: 1,
        changed: true,
        data: [
          {
            id: '2',
            at: '2026-10-06T04:30:00Z',
            userId: 'u-2',
            userName: 'Newer Branch B02',
            ip: '192.168.1.2',
            result: 'OK',
          },
        ],
      }),
    );
    await flush();

    expect(c.textContent).toContain('Newer Branch B02');

    // Later resolve older branch B01
    defB01.resolve(
      json({
        version: 1,
        changed: true,
        data: [
          {
            id: '1',
            at: '2026-10-06T02:30:00Z',
            userId: 'u-1',
            userName: 'Older Branch B01',
            ip: '192.168.1.1',
            result: 'OK',
          },
        ],
      }),
    );
    await flush();

    // Older response must be dropped, newer branch rows must stay
    expect(c.textContent).toContain('Newer Branch B02');
    expect(c.textContent).not.toContain('Older Branch B01');
  });

  it('renders localized result texts for each of the six known results, never the raw code', async () => {
    const results = [
      { code: 'OK', expected: 'สำเร็จ' },
      { code: 'BAD_PASSWORD', expected: 'รหัสผ่านไม่ถูกต้อง' },
      { code: 'UNKNOWN_USER', expected: 'ไม่พบบัญชีผู้ใช้' },
      { code: 'NOT_ACTIVE', expected: 'บัญชีไม่ได้เปิดใช้งาน' },
      { code: 'TEMP_EXPIRED', expected: 'รหัสผ่านชั่วคราวหมดอายุ' },
      { code: 'RATE_LIMITED', expected: 'ถูกจำกัดจำนวนครั้ง' },
    ];

    const rows = results.map((r, i) => ({
      id: String(i + 1),
      at: '2026-10-06T02:30:00Z',
      userId: `u-${i + 1}`,
      userName: `User ${r.code}`,
      ip: '127.0.0.1',
      result: r.code as any,
    }));

    handler = () => json({ version: 1, changed: true, data: rows });

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    const trs = c.querySelectorAll('tbody tr');
    expect(trs).toHaveLength(results.length);

    results.forEach((r, i) => {
      const resultCell = trs[i].children[2];
      expect(resultCell.textContent).toBe(r.expected);
      expect(resultCell.textContent).not.toContain(r.code);
    });
  });

  it('UNAVAILABLE result shows error.serverError text and not the raw code (FE controller ruling)', async () => {
    const rows = [
      {
        id: '1',
        at: '2026-10-06T02:30:00Z',
        userId: 'u-1',
        userName: 'Somchai',
        ip: '127.0.0.1',
        result: 'UNAVAILABLE' as any,
      },
    ];

    handler = () => json({ version: 1, changed: true, data: rows });

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    const resultCell = c.querySelector('tbody tr td:nth-child(3)');
    expect(resultCell?.textContent).toBe('เซิร์ฟเวอร์ขัดข้อง');
    expect(c.textContent).not.toContain('UNAVAILABLE');
  });

  it('unknown result code is shown as given', async () => {
    const rows = [
      {
        id: '1',
        at: '2026-10-06T02:30:00Z',
        userId: 'u-1',
        userName: 'Somchai',
        ip: '127.0.0.1',
        result: 'CUSTOM_UNKNOWN_CODE' as any,
      },
    ];

    handler = () => json({ version: 1, changed: true, data: rows });

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    const resultCell = c.querySelector('tbody tr td:nth-child(3)');
    expect(resultCell?.textContent).toBe('CUSTOM_UNKNOWN_CODE');
  });

  it('userId / userName null renders "—" in user cell', async () => {
    const rows = [
      {
        id: '1',
        at: '2026-10-06T02:30:00Z',
        userId: null,
        userName: null,
        ip: '127.0.0.1',
        result: 'OK' as const,
      },
    ];

    handler = () => json({ version: 1, changed: true, data: rows });

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    const userCell = c.querySelector('tbody tr td:nth-child(2)');
    expect(userCell?.textContent).toBe('—');
  });

  it('ip null renders "—" in ip cell', async () => {
    const rows = [
      {
        id: '1',
        at: '2026-10-06T02:30:00Z',
        userId: 'u-1',
        userName: 'Somchai',
        ip: null,
        result: 'OK' as const,
      },
    ];

    handler = () => json({ version: 1, changed: true, data: rows });

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    const ipCell = c.querySelector('tbody tr td:nth-child(4)');
    expect(ipCell?.textContent).toBe('—');
  });

  it('at is shown in Asia/Bangkok time', async () => {
    const rows = [
      {
        id: '1',
        at: '2026-10-06T02:30:00Z',
        userId: 'u-1',
        userName: 'Somchai',
        ip: '127.0.0.1',
        result: 'OK' as const,
      },
    ];

    handler = () => json({ version: 1, changed: true, data: rows });

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    const whenCell = c.querySelector('tbody tr td:nth-child(1)');
    expect(whenCell?.textContent).toContain('09:30');
  });

  it('no rows: shows loginsEmpty and no table', async () => {
    handler = () => json({ version: 1, changed: true, data: [] });

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    expect(c.querySelector('table')).toBeNull();
    expect(c.textContent).toContain('ไม่พบประวัติการเข้าสู่ระบบ');
  });

  it('refused read: shows wordedError and no table', async () => {
    handler = () => refusal(403, 'forbidden', 'Forbidden');

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    expect(c.querySelector('table')).toBeNull();
    expect(c.querySelector('[data-users-error]')).not.toBeNull();
  });

  it('tab is read-only: contains no input, button, select, textarea, or contenteditable elements', async () => {
    const rows = [
      {
        id: '1',
        at: '2026-10-06T02:30:00Z',
        userId: 'u-1',
        userName: 'Somchai',
        ip: '127.0.0.1',
        result: 'OK' as const,
      },
    ];
    handler = () => json({ version: 1, changed: true, data: rows });

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    const interactive = c.querySelectorAll('input, button, select, textarea, [contenteditable]');
    expect(interactive).toHaveLength(0);
  });

  it('edge case: device in UTC still shows Bangkok time 09:30', async () => {
    const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } })
      .process.env;
    const oldTZ = env.TZ;
    env.TZ = 'UTC';
    try {
      const rows = [
        {
          id: '1',
          at: '2026-10-06T02:30:00Z',
          userId: 'u-1',
          userName: 'Somchai',
          ip: '127.0.0.1',
          result: 'OK' as const,
        },
      ];
      handler = () => json({ version: 1, changed: true, data: rows });

      const [shown] = createSignal(true);
      const [branch] = createSignal('XC-B01');
      const c = mount(shown, branch);
      await flush();

      const whenCell = c.querySelector('tbody tr td:nth-child(1)');
      expect(whenCell?.textContent).toContain('09:30');
    } finally {
      env.TZ = oldTZ;
    }
  });

  it('edge case: userName containing <script> is rendered as text and no script element is in the DOM', async () => {
    const rows = [
      {
        id: '1',
        at: '2026-10-06T02:30:00Z',
        userId: 'u-1',
        userName: '<script>alert("xss")</script>',
        ip: '127.0.0.1',
        result: 'OK' as const,
      },
    ];
    handler = () => json({ version: 1, changed: true, data: rows });

    const [shown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    const c = mount(shown, branch);
    await flush();

    expect(c.querySelector('script')).toBeNull();
    expect(c.textContent).toContain('<script>alert("xss")</script>');
  });

  it('edge case: shown() going true, false, true triggers two reads', async () => {
    handler = () => json({ version: 1, changed: true, data: [] });

    const [shown, setShown] = createSignal(true);
    const [branch] = createSignal('XC-B01');
    mount(shown, branch);
    await flush();

    expect(calls.filter((c) => c.url.includes('admin_login_history'))).toHaveLength(1);

    setShown(false);
    await flush();
    expect(calls.filter((c) => c.url.includes('admin_login_history'))).toHaveLength(1);

    setShown(true);
    await flush();
    expect(calls.filter((c) => c.url.includes('admin_login_history'))).toHaveLength(2);
  });
});
