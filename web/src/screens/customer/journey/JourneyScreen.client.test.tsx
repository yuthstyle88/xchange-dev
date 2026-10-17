// RR-3 (#2034): C09 JourneyScreen reads my_queue_ticket and my_planner_drafts through ApiClient.view & wordedError
// Design: docs/design/xchange-v3-web.md §12.25, row RR-3.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import type { MyPlannerDraftsData, PlannerDraftDto } from '../../../shared/api/generated/my_planner_drafts';
import type { MyQueueTicketData } from '../../../shared/api/generated/my_queue_ticket';
import { setAudience, setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import { JourneyScreen } from './JourneyScreen';
import { th as screenTh } from './i18n';

const SAMPLE_TICKET: MyQueueTicketData = {
  ticketId: 'tkt-rr3-01',
  branch: 'SIAM',
  displayNumber: 'A088',
  serviceType: 'BOOKING',
  status: 'WAITING',
  ticketsAhead: 2,
  estimateSeconds: 300,
  estimateLowSeconds: 240,
  estimateHighSeconds: 360,
  estimateAsOf: '2026-10-15T10:00:00Z',
};

const SAMPLE_DRAFT: PlannerDraftDto = {
  id: 'draft-rr3-01',
  branch: 'SIAM',
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '1200.00' },
  version: 1,
  expiresAt: '2026-10-22T10:00:00Z',
  createdAt: '2026-10-15T10:00:00Z',
  updatedAt: '2026-10-15T10:00:00Z',
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('RR-3 C09 JourneyScreen ApiClient.view & wordedError (#2034 §9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeAll(() => warmModules(() => import('./JourneyScreen')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  it("row 1: the read goes through ApiClient.view('my_queue_ticket', { branch, filter: { ticketId } }) and ApiClient.view('my_planner_drafts')", async () => {
    const viewSpy = vi.spyOn(ApiClient.prototype, 'view');
    const seenUrls: string[] = [];

    const testFetch = async (url: string): Promise<Response> => {
      seenUrls.push(url);
      if (url.startsWith('/api/v3/views/my_queue_ticket')) {
        return json(200, { version: 1, changed: true, data: SAMPLE_TICKET });
      }
      if (url.startsWith('/api/v3/views/my_planner_drafts')) {
        const draftsData: MyPlannerDraftsData = { drafts: [SAMPLE_DRAFT] };
        return json(200, { version: 1, changed: true, data: draftsData });
      }
      return json(404, { error: { code: 'notFound', message: url } });
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} search="?ticketId=tkt-rr3-01&branch=SIAM" />,
      host,
    );
    await flush();

    expect(viewSpy).toHaveBeenCalledWith('my_queue_ticket', {
      branch: 'SIAM',
      filter: { ticketId: 'tkt-rr3-01' },
    });
    expect(viewSpy).toHaveBeenCalledWith('my_planner_drafts');
    expect(seenUrls).toContain('/api/v3/views/my_queue_ticket?branch=SIAM&ticketId=tkt-rr3-01');
    expect(seenUrls).toContain('/api/v3/views/my_planner_drafts');
  });

  it("row 2: 500 {code:'roleNotAllowedHere'} shows th['error.serverError']", async () => {
    const testFetch = async (): Promise<Response> =>
      json(500, {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Internal database failure disguised as role error',
        },
      });

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-500" branch="SIAM" />,
      host,
    );
    await flush();

    const queueErrorDetail = host.querySelector('[data-testid="queue-error-detail"]');
    const plansErrorDetail = host.querySelector('[data-testid="plans-error-detail"]');

    expect(queueErrorDetail?.textContent).toBe(sharedTh['error.serverError']);
    expect(plansErrorDetail?.textContent).toBe(sharedTh['error.serverError']);
    expect(host.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    expect(host.textContent).not.toContain('Internal database failure disguised as role error');
  });

  it('row 3: a 4xx keyed refusal shows its key text (and not always serverError)', async () => {
    const testFetch = async (url: string): Promise<Response> => {
      if (url.startsWith('/api/v3/views/my_queue_ticket')) {
        return json(403, {
          error: {
            code: 'roleNotAllowedHere',
            message: 'raw 403 wire message for queue',
          },
        });
      }
      return json(400, {
        error: {
          code: 'noSuchBranch',
          message: 'raw 400 wire message for drafts',
        },
      });
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-403" branch="SIAM" />,
      host,
    );
    await flush();

    const queueErrorDetail = host.querySelector('[data-testid="queue-error-detail"]');
    const plansErrorDetail = host.querySelector('[data-testid="plans-error-detail"]');

    expect(queueErrorDetail?.textContent).toBe(sharedTh['error.roleNotAllowedHere']);
    expect(plansErrorDetail?.textContent).toBe(sharedTh['error.noSuchBranch']);
    expect(host.textContent).not.toContain(sharedTh['error.serverError']);
    expect(host.textContent).not.toContain('raw 403 wire message for queue');
    expect(host.textContent).not.toContain('raw 400 wire message for drafts');
  });

  it('row 4: network TypeError shows error.network', async () => {
    const testFetch = async (): Promise<Response> => {
      throw new TypeError('Failed to fetch');
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-net" branch="SIAM" />,
      host,
    );
    await flush();

    const queueErrorDetail = host.querySelector('[data-testid="queue-error-detail"]');
    const plansErrorDetail = host.querySelector('[data-testid="plans-error-detail"]');

    expect(queueErrorDetail?.textContent).toBe(sharedTh['error.network']);
    expect(plansErrorDetail?.textContent).toBe(sharedTh['error.network']);
    expect(host.textContent).not.toContain(sharedTh['error.serverError']);
    expect(host.textContent).not.toContain('Failed to fetch');

    setLang('en');
    await flush();
    expect(host.querySelector('[data-testid="queue-error-detail"]')?.textContent).toBe(
      sharedEn['error.network'],
    );
    expect(host.querySelector('[data-testid="plans-error-detail"]')?.textContent).toBe(
      sharedEn['error.network'],
    );
  });

  it('row 5: 401 routes to sign-in (/app/sign-in) with no error text', async () => {
    const testFetch = async (): Promise<Response> =>
      json(401, {
        error: {
          code: 'signIn',
          message: 'Please sign in',
        },
      });

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-401" branch="SIAM" />,
      host,
    );
    await flush();

    const signInLinks = Array.from(
      host.querySelectorAll('a.xc-customer-signin[href="/app/sign-in"]'),
    );
    expect(signInLinks.length).toBeGreaterThanOrEqual(2);
    expect(host.querySelector('[data-testid="queue-unauthorized"]')).not.toBeNull();
    expect(host.querySelector('[data-testid="plans-unauthorized"]')).not.toBeNull();

    // No error alert or error text is shown
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('.xc-journey-error')).toBeNull();
    expect(host.textContent).not.toContain(screenTh.errorQueue);
    expect(host.textContent).not.toContain(screenTh.errorPlans);
    expect(host.textContent).not.toContain(sharedTh['error.serverError']);
  });

  it('row 6: success renders ticket and planner drafts as today', async () => {
    const testFetch = async (url: string): Promise<Response> => {
      if (url.startsWith('/api/v3/views/my_queue_ticket')) {
        return json(200, { version: 1, changed: true, data: SAMPLE_TICKET });
      }
      if (url.startsWith('/api/v3/views/my_planner_drafts')) {
        const draftsData: MyPlannerDraftsData = { drafts: [SAMPLE_DRAFT] };
        return json(200, { version: 1, changed: true, data: draftsData });
      }
      return json(404, { error: { code: 'notFound', message: url } });
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-rr3-01" branch="SIAM" />,
      host,
    );
    await flush();

    expect(host.querySelector('[data-testid="ticket-card"]')).not.toBeNull();
    expect(host.querySelector('[data-testid="ticket-number"]')?.textContent).toBe('A088');
    expect(host.querySelector('.ticket-branch')?.textContent).toBe('SIAM');
    expect(host.querySelector('[data-testid="tickets-ahead"]')?.textContent).toContain('2');
    expect(host.querySelector('[data-testid="wait-estimate"]')?.textContent).toContain('4 - 6');

    const draftCards = host.querySelectorAll('[data-testid="draft-card"]');
    expect(draftCards.length).toBe(1);
    expect(host.textContent).toContain('1,200.00 USD');
    expect(host.textContent).toContain(screenTh.customerBuy);
  });
});
