import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PlannerDraftDto } from '../../../shared/api/generated/my_planner_drafts';
import type { MyQueueTicketData } from '../../../shared/api/generated/my_queue_ticket';
import { setAudience, setLang } from '../../../shared/i18n';
import * as i18n from './i18n';
import { JourneyScreen } from './JourneyScreen';

interface NodeFs {
  readFileSync(path: string, encoding: 'utf8'): string;
  existsSync(path: string): boolean;
}

let fs: NodeFs;
let dir = '';

beforeAll(async () => {
  const fsName = 'node:fs';
  fs = (await import(/* @vite-ignore */ fsName)) as NodeFs;
  dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
});

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function flushMicrotasks(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe('C09 Customer Journey Screen (APP-13, design customer-app.md §4.5, §9.1)', () => {
  let host: HTMLElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  // --------------------------------------------------------------------------
  // 1. Page and Main configuration
  // --------------------------------------------------------------------------

  it('web/pages/app-journey.html is the anyone page at /app/journey that loads this screen (design §4.5)', () => {
    const pagePath = dir + '../../../../pages/app-journey.html';
    expect(fs.existsSync(pagePath)).toBe(true);
    const pageHtml = fs.readFileSync(pagePath, 'utf8');

    expect(pageHtml).toContain('<meta name="xc-path" content="/app/journey">');
    expect(pageHtml).toContain('<meta name="xc-gate" content="anyone">');
    expect(pageHtml).toContain('src="/src/screens/customer/journey/main.tsx"');
  });

  it("main.tsx declares setAudience('customer') and wraps in CustomerShell route='/app/journey' (design §4.5, shell contract)", () => {
    const mainPath = dir + 'main.tsx';
    expect(fs.existsSync(mainPath)).toBe(true);
    const mainCode = fs.readFileSync(mainPath, 'utf8');

    expect(mainCode).toContain("setAudience('customer')");
    expect(mainCode).toContain("CustomerShell route=\"/app/journey\"");
    expect(mainCode).toContain("<JourneyScreen");
  });

  // --------------------------------------------------------------------------
  // 2. i18n completeness and Chinese check (L7)
  // --------------------------------------------------------------------------

  it('i18n.ts exports complete th, en, zh tables with identical keys and matching placeholders (L4, L7)', () => {
    const { th, en, zh } = i18n;
    expect(th).toBeDefined();
    expect(en).toBeDefined();
    expect(zh).toBeDefined();

    const thKeys = Object.keys(th).sort();
    const enKeys = Object.keys(en).sort();
    const zhKeys = Object.keys(zh).sort();

    expect(thKeys).toEqual(enKeys);
    expect(thKeys).toEqual(zhKeys);

    for (const key of thKeys) {
      const thVal = th[key as keyof typeof th];
      const enVal = en[key as keyof typeof en];
      const zhVal = zh[key as keyof typeof zh];

      expect(typeof thVal).toBe('string');
      expect(typeof enVal).toBe('string');
      expect(typeof zhVal).toBe('string');

      expect(thVal.trim().length).toBeGreaterThan(0);
      expect(enVal.trim().length).toBeGreaterThan(0);
      expect(zhVal.trim().length).toBeGreaterThan(0);

      const thPlaceholders = [...thVal.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      const enPlaceholders = [...enVal.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      const zhPlaceholders = [...zhVal.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

      expect(thPlaceholders).toEqual(enPlaceholders);
      expect(thPlaceholders).toEqual(zhPlaceholders);
    }
  });

  // --------------------------------------------------------------------------
  // 3. Language Switch
  // --------------------------------------------------------------------------

  it('switches text dynamically across TH / EN / 中文 via shared i18n setter', async () => {
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(() => <JourneyScreen fetch={fetchMock} />, host);
    await flushMicrotasks();

    // The shell owns the language switch (TH / EN / 中文), screen must not draw its own
    expect(host.querySelector('.xc-journey-actions')).toBeNull();
    expect(host.querySelector('.xc-lang-switch')).toBeNull();

    // Initially TH
    expect(host.querySelector('h1')?.textContent).toBe(i18n.th.title);
    expect(host.querySelector('.xc-journey-subtitle')?.textContent).toBe(i18n.th.subtitle);

    // Switch to EN
    setLang('en');
    await flushMicrotasks();
    expect(host.querySelector('h1')?.textContent).toBe(i18n.en.title);
    expect(host.querySelector('.xc-journey-subtitle')?.textContent).toBe(i18n.en.subtitle);

    // Switch to ZH
    setLang('zh');
    await flushMicrotasks();
    expect(host.querySelector('h1')?.textContent).toBe(i18n.zh.title);
    expect(host.querySelector('.xc-journey-subtitle')?.textContent).toBe(i18n.zh.subtitle);
  });

  // --------------------------------------------------------------------------
  // 4. Saved Plans (Planner Drafts) States
  // --------------------------------------------------------------------------

  it('shows loading state while fetching saved plans', () => {
    let pendingResolve: (r: Response) => void;
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_planner_drafts')) {
        return new Promise<Response>((resolve) => {
          pendingResolve = resolve;
        });
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(() => <JourneyScreen fetch={fetchMock} />, host);

    expect(host.textContent).toContain(i18n.th.loadingPlans);

    pendingResolve!(jsonResponse(200, { drafts: [] }));
  });

  it('shows empty state when planner drafts list is empty', async () => {
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(() => <JourneyScreen fetch={fetchMock} />, host);
    await flushMicrotasks();

    const emptyEl = host.querySelector('[data-testid="plans-empty"]');
    expect(emptyEl).not.toBeNull();
    expect(emptyEl?.textContent).toBe(i18n.th.emptyPlans);
  });

  it('shows error state and retries on failure for planner drafts', async () => {
    let attempts = 0;
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_planner_drafts')) {
        attempts++;
        if (attempts === 1) {
          return Promise.resolve(jsonResponse(500, { code: 'serverError' }));
        }
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(() => <JourneyScreen fetch={fetchMock} />, host);
    await flushMicrotasks();

    // Verify error state
    expect(host.textContent).toContain(i18n.th.errorPlans);
    const retryBtn = host.querySelector('.xc-journey-section:nth-of-type(2) .xc-journey-retry-btn') as HTMLButtonElement;
    expect(retryBtn).not.toBeNull();

    // Click retry
    retryBtn.click();
    await flushMicrotasks();

    expect(attempts).toBe(2);
    expect(host.textContent).toContain(i18n.th.emptyPlans);
  });

  it('renders saved planner drafts with MoneyText and amounts do not change with locale', async () => {
    const sampleDrafts: PlannerDraftDto[] = [
      {
        id: 'draft-1',
        branch: 'MAIN',
        currency: 'USD',
        customerSide: 'CUSTOMER_BUY',
        amount: { quantity: '500.00' },
        version: 1,
        expiresAt: '2026-10-20T12:00:00Z',
        createdAt: '2026-10-13T12:00:00Z',
        updatedAt: '2026-10-13T12:00:00Z',
      },
      {
        id: 'draft-2',
        branch: null,
        currency: 'JPY',
        customerSide: 'CUSTOMER_SELL',
        amount: { budgetThb: '20000.00' },
        version: 1,
        expiresAt: '2026-10-20T12:00:00Z',
        createdAt: '2026-10-13T12:00:00Z',
        updatedAt: '2026-10-13T12:00:00Z',
      },
    ];

    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: sampleDrafts }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(() => <JourneyScreen fetch={fetchMock} />, host);
    await flushMicrotasks();

    const draftCards = host.querySelectorAll('[data-testid="draft-card"]');
    expect(draftCards.length).toBe(2);

    // Amounts format check
    expect(host.textContent).toContain('500.00 USD');
    expect(host.textContent).toContain('20,000.00 THB');

    // Customer side texts check in Thai
    expect(host.textContent).toContain(i18n.th.customerBuy);
    expect(host.textContent).toContain(i18n.th.customerSell);

    // Switch to English: amounts remain unchanged
    setLang('en');
    await flushMicrotasks();
    expect(host.textContent).toContain('500.00 USD');
    expect(host.textContent).toContain('20,000.00 THB');
    expect(host.textContent).toContain(i18n.en.customerBuy);

    // Switch to Chinese: amounts remain unchanged
    setLang('zh');
    await flushMicrotasks();
    expect(host.textContent).toContain('500.00 USD');
    expect(host.textContent).toContain('20,000.00 THB');
    expect(host.textContent).toContain(i18n.zh.customerBuy);
  });

  // --------------------------------------------------------------------------
  // 5. Queue Ticket States and Wait Estimate Rule
  // --------------------------------------------------------------------------

  it('shows empty queue ticket state when no ticket is active', async () => {
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(() => <JourneyScreen fetch={fetchMock} />, host);
    await flushMicrotasks();

    const emptyQueue = host.querySelector('[data-testid="queue-empty"]');
    expect(emptyQueue).not.toBeNull();
    expect(emptyQueue?.textContent).toBe(i18n.th.noQueueTicket);
  });

  it('shows error state and retries on queue ticket fetch failure', async () => {
    let attempts = 0;
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_queue_ticket')) {
        attempts++;
        if (attempts === 1) {
          return Promise.resolve(jsonResponse(500, { code: 'serverError' }));
        }
        return Promise.resolve(jsonResponse(404, {}));
      }
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(
      () => <JourneyScreen fetch={fetchMock} ticketId="tkt-123" branch="MAIN" />,
      host,
    );
    await flushMicrotasks();

    expect(host.textContent).toContain(i18n.th.errorQueue);
    const retryBtn = host.querySelector('.xc-journey-section:nth-of-type(1) .xc-journey-retry-btn') as HTMLButtonElement;
    expect(retryBtn).not.toBeNull();

    retryBtn.click();
    await flushMicrotasks();

    expect(attempts).toBe(2);
  });

  it('C09 shows "unknown" in each language when wait estimate is NULL, never a number (design §9.1 APP-13, issue #1005 §3)', async () => {
    const ticketWithNullEstimate: MyQueueTicketData = {
      ticketId: 'tkt-null-est',
      branch: 'MAIN',
      displayNumber: 'A042',
      serviceType: 'WALK_IN',
      status: 'WAITING',
      ticketsAhead: 3,
      estimateSeconds: null,
      estimateLowSeconds: null,
      estimateHighSeconds: null,
      estimateAsOf: null,
    };

    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_queue_ticket')) {
        return Promise.resolve(jsonResponse(200, { data: ticketWithNullEstimate }));
      }
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(
      () => <JourneyScreen fetch={fetchMock} ticketId="tkt-null-est" branch="MAIN" />,
      host,
    );
    await flushMicrotasks();

    const waitEstEl = host.querySelector('[data-testid="wait-estimate"]');
    expect(waitEstEl).not.toBeNull();

    // Thai: "ไม่ระบุ", no digits
    expect(waitEstEl?.textContent?.trim()).toBe(i18n.th.unknown);
    expect(waitEstEl?.textContent).not.toMatch(/\d+/);

    // English: "unknown", no digits
    setLang('en');
    await flushMicrotasks();
    expect(host.querySelector('[data-testid="wait-estimate"]')?.textContent?.trim()).toBe(i18n.en.unknown);
    expect(host.querySelector('[data-testid="wait-estimate"]')?.textContent).not.toMatch(/\d+/);

    // Chinese: "未知", no digits
    setLang('zh');
    await flushMicrotasks();
    expect(host.querySelector('[data-testid="wait-estimate"]')?.textContent?.trim()).toBe(i18n.zh.unknown);
    expect(host.querySelector('[data-testid="wait-estimate"]')?.textContent).not.toMatch(/\d+/);
  });

  it('shows sign-in prompt truthfully when queue ticket fetch returns 401 (signed out)', async () => {
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_queue_ticket')) {
        return Promise.resolve(jsonResponse(401, { code: 'unauthorized' }));
      }
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(
      () => <JourneyScreen fetch={fetchMock} ticketId="tkt-401" branch="MAIN" />,
      host,
    );
    await flushMicrotasks();

    expect(host.textContent).toContain(i18n.th.signInToViewTicket);
    expect(host.textContent).not.toContain(i18n.th.noQueueTicket);

    setLang('en');
    await flushMicrotasks();
    expect(host.textContent).toContain(i18n.en.signInToViewTicket);

    setLang('zh');
    await flushMicrotasks();
    expect(host.textContent).toContain(i18n.zh.signInToViewTicket);
  });

  it('shows empty queue ticket state ("no ticket") when queue ticket fetch returns 403 (outOfScope)', async () => {
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_queue_ticket')) {
        return Promise.resolve(jsonResponse(403, { code: 'outOfScope' }));
      }
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(
      () => <JourneyScreen fetch={fetchMock} ticketId="tkt-other-cx" branch="MAIN" />,
      host,
    );
    await flushMicrotasks();

    // 403 outOfScope must show "no ticket", giving no oracle or hint that the ticket exists
    const emptyQueue = host.querySelector('[data-testid="queue-empty"]');
    expect(emptyQueue).not.toBeNull();
    expect(emptyQueue?.textContent).toBe(i18n.th.noQueueTicket);
    // Must NOT show sign-in prompt or error
    expect(host.querySelector('[data-testid="queue-unauthorized"]')).toBeNull();
    expect(host.textContent).not.toContain(i18n.th.signInToViewTicket);

    // Also in English:
    setLang('en');
    await flushMicrotasks();
    expect(host.querySelector('[data-testid="queue-empty"]')?.textContent).toBe(i18n.en.noQueueTicket);
    expect(host.textContent).not.toContain(i18n.en.signInToViewTicket);

    // Also in Chinese:
    setLang('zh');
    await flushMicrotasks();
    expect(host.querySelector('[data-testid="queue-empty"]')?.textContent).toBe(i18n.zh.noQueueTicket);
    expect(host.textContent).not.toContain(i18n.zh.signInToViewTicket);
  });

  it('extracts ticketId and branch from search query params without localStorage', async () => {
    let requestedUrl = '';
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_queue_ticket')) {
        requestedUrl = url;
        return Promise.resolve(jsonResponse(404, {}));
      }
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(
      () => <JourneyScreen fetch={fetchMock} search="?ticketId=T-999&branch=SIAM" />,
      host,
    );
    await flushMicrotasks();

    expect(requestedUrl).toContain('branch=SIAM');
    expect(requestedUrl).toContain('ticketId=T-999');
  });

  it('shows estimated minutes when wait estimate is a number', async () => {
    const ticketWithEstimate: MyQueueTicketData = {
      ticketId: 'tkt-with-est',
      branch: 'MAIN',
      displayNumber: 'B015',
      serviceType: 'BOOKING',
      status: 'WAITING',
      ticketsAhead: 2,
      estimateSeconds: 300,
      estimateLowSeconds: 240,
      estimateHighSeconds: 360,
      estimateAsOf: '2026-10-13T12:00:00Z',
    };

    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/my_queue_ticket')) {
        return Promise.resolve(jsonResponse(200, { data: ticketWithEstimate }));
      }
      if (url.includes('/my_planner_drafts')) {
        return Promise.resolve(jsonResponse(200, { drafts: [] }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(
      () => <JourneyScreen fetch={fetchMock} ticketId="tkt-with-est" branch="MAIN" />,
      host,
    );
    await flushMicrotasks();

    expect(host.querySelector('[data-testid="ticket-number"]')?.textContent).toBe('B015');
    expect(host.querySelector('[data-testid="tickets-ahead"]')?.textContent).toContain('2');

    const waitEstEl = host.querySelector('[data-testid="wait-estimate"]');
    // Low: 4, High: 6
    expect(waitEstEl?.textContent).toContain('4 - 6');
  });
});
