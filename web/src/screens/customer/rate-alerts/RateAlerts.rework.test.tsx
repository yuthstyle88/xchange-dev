import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { RateAlertsScreen } from './RateAlerts';

const disposers: Array<() => void> = [];

const SAMPLE_BRANCHES: PublicBranchesData = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสีลม คอมเพล็กซ์',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
    {
      code: 'XC-B02',
      name: 'สาขาสนามบินดอนเมือง',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: false,
    },
  ],
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function refusal(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

let hostReply: (() => Response) | null;
let branchesReply: (() => Response) | null;
let myAlertsReply: (() => Response) | null;

async function fakeFetch(url: string, _init?: RequestInit): Promise<Response> {
  if (url === '/api/public/host') {
    if (hostReply) return hostReply();
    return json({ org: 'XC-ORG' });
  }

  if (url.includes('/api/public/orgs/')) {
    if (branchesReply) return branchesReply();
    return json(SAMPLE_BRANCHES);
  }

  if (url.includes('/api/v3/views/my_rate_alerts')) {
    if (myAlertsReply) return myAlertsReply();
    return json({ version: 1, changed: true, data: { alerts: [] } });
  }

  return refusal(404, 'notFound', 'not found');
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function mount(props: { org?: string } = {}) {
  const container = document.createElement('div');
  document.body.appendChild(container);

  const dispose = render(
    () => <RateAlertsScreen fetch={fakeFetch} org={props.org} />,
    container,
  );
  disposers.push(dispose);
  return { container };
}

beforeEach(() => {
  hostReply = null;
  branchesReply = null;
  myAlertsReply = null;
  document.body.innerHTML = '';
  setLang('th');
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('108-Plaza/xchange#1717: RateAlerts branch list failure shows worded error', () => {
  it('shows worded error in Thai when branches endpoint fails with 500', async () => {
    branchesReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    const { container } = mount();
    await flush();

    const alerts = Array.from(container.querySelectorAll('[role="alert"]'));
    const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    const found = alerts.some((a) => a.textContent?.includes(expected));
    expect(found, `Expected role="alert" containing "${expected}"`).toBe(true);
  });

  it('shows worded error when branches endpoint returns 403 refusal (known code)', async () => {
    branchesReply = () => refusal(403, 'roleNotAllowedHere', 'raw wire message');
    const { container } = mount();
    await flush();

    const alerts = Array.from(container.querySelectorAll('[role="alert"]'));
    const expected = errorText('roleNotAllowedHere', 'เซิร์ฟเวอร์ขัดข้อง');
    const found = alerts.some((a) => a.textContent?.includes(expected));
    expect(found, `Expected role="alert" containing "${expected}"`).toBe(true);
    expect(alerts.some((a) => a.textContent?.includes('raw wire message'))).toBe(false);
  });

  it('shows server sentence for 400 with unknown code without leaking raw code', async () => {
    branchesReply = () => refusal(400, 'somethingNew', 'raw wire message');
    const { container } = mount();
    await flush();

    const alerts = Array.from(container.querySelectorAll('[role="alert"]'));
    const expected = 'raw wire message';
    const found = alerts.some((a) => a.textContent?.includes(expected));
    expect(found, `Expected role="alert" containing "${expected}"`).toBe(true);
    expect(alerts.some((a) => a.textContent?.includes('somethingNew'))).toBe(false);
  });

  it('shows network worded error when fetch rejects with TypeError without leaking Failed to fetch', async () => {
    branchesReply = () => {
      throw new TypeError('Failed to fetch');
    };
    const { container } = mount();
    await flush();

    const alerts = Array.from(container.querySelectorAll('[role="alert"]'));
    const expected = errorText('network', 'เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่');
    const found = alerts.some((a) => a.textContent?.includes(expected));
    expect(found, `Expected role="alert" containing "${expected}"`).toBe(true);
    expect(alerts.some((a) => a.textContent?.includes('Failed to fetch'))).toBe(false);
  });

  it('shows worded error in English when setLang("en") is active', async () => {
    setLang('en');
    branchesReply = () => refusal(500, 'serverError', 'Server error');
    const { container } = mount();
    await flush();

    const alerts = Array.from(container.querySelectorAll('[role="alert"]'));
    const expected = errorText('serverError', 'Server error');
    const found = alerts.some((a) => a.textContent?.includes(expected));
    expect(found, `Expected role="alert" containing "${expected}" in English`).toBe(true);
  });

  it('shows worded error when /api/public/host fails with 500', async () => {
    hostReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    const { container } = mount();
    await flush();

    const alerts = Array.from(container.querySelectorAll('[role="alert"]'));
    const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    const found = alerts.some((a) => a.textContent?.includes(expected));
    expect(found, `Expected role="alert" containing "${expected}" when host fails`).toBe(true);
  });

  it('retains stale branches while showing error if subsequent branches read fails', async () => {
    let callCount = 0;
    branchesReply = () => {
      callCount++;
      if (callCount === 1) {
        return json(SAMPLE_BRANCHES);
      }
      return refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    };
    // Cause my_rate_alerts to fail so retry button is available
    myAlertsReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');

    const { container } = mount({ org: 'XC-ORG' });
    await flush();

    // First load succeeded: select has 2 branch options
    const select = container.querySelector<HTMLSelectElement>('#alert-branch');
    expect(select?.querySelectorAll('option').length).toBe(2);

    // Trigger retry
    const retryBtn = container.querySelector<HTMLButtonElement>('.xc-alerts-retry-btn');
    expect(retryBtn).not.toBeNull();
    retryBtn!.click();
    await flush();

    // Second call failed with 500: stale branches remain in select
    expect(select?.querySelectorAll('option').length).toBe(2);

    // Worded error is displayed
    const alerts = Array.from(container.querySelectorAll('[role="alert"]'));
    const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
    expect(alerts.some((a) => a.textContent?.includes(expected))).toBe(true);
  });
});
