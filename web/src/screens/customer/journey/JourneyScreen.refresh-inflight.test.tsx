import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PlannerDraftDto } from '../../../shared/api/generated/my_planner_drafts';
import type { MyQueueTicketData } from '../../../shared/api/generated/my_queue_ticket';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { JourneyScreen } from './JourneyScreen';

const WAITING_TICKET: MyQueueTicketData = {
  ticketId: 'tkt-01',
  branch: 'SIAM',
  displayNumber: 'A001',
  serviceType: 'BOOKING',
  status: 'WAITING',
  ticketsAhead: 5,
  estimateSeconds: 300,
  estimateLowSeconds: 240,
  estimateHighSeconds: 360,
  estimateAsOf: '2026-10-15T10:00:00Z',
};

const CALLED_TICKET: MyQueueTicketData = {
  ticketId: 'tkt-01',
  branch: 'SIAM',
  displayNumber: 'A001',
  serviceType: 'BOOKING',
  status: 'CALLED',
  ticketsAhead: 0,
  estimateSeconds: 0,
  estimateLowSeconds: 0,
  estimateHighSeconds: 0,
  estimateAsOf: '2026-10-15T10:05:00Z',
};

const DRAFT_1: PlannerDraftDto = {
  id: 'draft-01',
  branch: 'SIAM',
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '100.00' },
  version: 1,
  expiresAt: '2026-10-22T10:00:00Z',
  createdAt: '2026-10-15T10:00:00Z',
  updatedAt: '2026-10-15T10:00:00Z',
};

const DRAFT_2: PlannerDraftDto = {
  id: 'draft-02',
  branch: 'SIAM',
  currency: 'JPY',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '10000.00' },
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

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function dispatchClick(el: HTMLElement): void {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
}

function getQueueRefreshButton(host: HTMLElement): HTMLButtonElement {
  return host.querySelector<HTMLButtonElement>('[aria-labelledby="queue-section-heading"] .xc-journey-refresh-btn')!;
}

function getDraftsRefreshButton(host: HTMLElement): HTMLButtonElement {
  return host.querySelector<HTMLButtonElement>('[aria-labelledby="plans-section-heading"] .xc-journey-refresh-btn')!;
}

describe('#2667: Customer journey refresh buttons and in-flight sequence guards', () => {
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

  it('row 1: Queue Refresh clicked, read in flight -> The queue Refresh button is disabled', async () => {
    const queueDeferred = deferred<Response>();
    let queueCallCount = 0;

    const testFetch = async (url: string): Promise<Response> => {
      if (url.startsWith('/api/v3/views/my_queue_ticket')) {
        queueCallCount++;
        if (queueCallCount === 1) {
          return json(200, { version: 1, changed: true, data: WAITING_TICKET });
        }
        return queueDeferred.promise;
      }
      if (url.startsWith('/api/v3/views/my_planner_drafts')) {
        return json(200, { version: 1, changed: true, data: { drafts: [DRAFT_1] } });
      }
      return json(404, {});
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-01" branch="SIAM" />,
      host,
    );
    await flush();

    const queueBtn = getQueueRefreshButton(host);
    expect(queueBtn.disabled).toBe(false);

    queueBtn.click();
    await flush();

    expect(queueBtn.disabled).toBe(true);

    queueDeferred.resolve(json(200, { version: 1, changed: true, data: WAITING_TICKET }));
    await flush();

    expect(queueBtn.disabled).toBe(false);
  });

  it('row 2: Two queue reads in flight; the later answers CALLED first, then the earlier answers WAITING -> CALLED is shown', async () => {
    const read1Deferred = deferred<Response>();
    const read2Deferred = deferred<Response>();
    let queueCallCount = 0;

    const testFetch = async (url: string): Promise<Response> => {
      if (url.startsWith('/api/v3/views/my_queue_ticket')) {
        queueCallCount++;
        if (queueCallCount === 1) {
          return read1Deferred.promise;
        }
        if (queueCallCount === 2) {
          return read2Deferred.promise;
        }
      }
      if (url.startsWith('/api/v3/views/my_planner_drafts')) {
        return json(200, { version: 1, changed: true, data: { drafts: [] } });
      }
      return json(404, {});
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-01" branch="SIAM" />,
      host,
    );
    await flush();

    const queueBtn = getQueueRefreshButton(host);

    // Overlapping click starts read 2 while mount read 1 is in flight
    queueBtn.removeAttribute('disabled');
    dispatchClick(queueBtn);
    await flush();

    // Later read (read 2) answers first with CALLED
    read2Deferred.resolve(json(200, { version: 2, changed: true, data: CALLED_TICKET }));
    await flush();

    expect(host.querySelector('[data-testid="ticket-status"]')?.className).toContain(
      'xc-journey-badge-called',
    );

    // Earlier read (read 1) answers second with WAITING
    read1Deferred.resolve(json(200, { version: 1, changed: true, data: WAITING_TICKET }));
    await flush();

    // Must still show CALLED
    expect(host.querySelector('[data-testid="ticket-status"]')?.className).toContain(
      'xc-journey-badge-called',
    );
  });

  it('row 3: Drafts Refresh clicked, read in flight -> The drafts Refresh button is disabled', async () => {
    const draftsDeferred = deferred<Response>();
    let draftsCallCount = 0;

    const testFetch = async (url: string): Promise<Response> => {
      if (url.startsWith('/api/v3/views/my_queue_ticket')) {
        return json(200, { version: 1, changed: true, data: WAITING_TICKET });
      }
      if (url.startsWith('/api/v3/views/my_planner_drafts')) {
        draftsCallCount++;
        if (draftsCallCount === 1) {
          return json(200, { version: 1, changed: true, data: { drafts: [DRAFT_1] } });
        }
        return draftsDeferred.promise;
      }
      return json(404, {});
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-01" branch="SIAM" />,
      host,
    );
    await flush();

    const draftsBtn = getDraftsRefreshButton(host);
    expect(draftsBtn.disabled).toBe(false);

    draftsBtn.click();
    await flush();

    expect(draftsBtn.disabled).toBe(true);

    draftsDeferred.resolve(json(200, { version: 1, changed: true, data: { drafts: [DRAFT_1] } }));
    await flush();

    expect(draftsBtn.disabled).toBe(false);
  });

  it('row 4: Two drafts reads in flight; the later answers 2 drafts, then the earlier answers 1 draft -> 2 drafts are shown', async () => {
    const read1Deferred = deferred<Response>();
    const read2Deferred = deferred<Response>();
    let draftsCallCount = 0;

    const testFetch = async (url: string): Promise<Response> => {
      if (url.startsWith('/api/v3/views/my_queue_ticket')) {
        return json(200, { version: 1, changed: true, data: WAITING_TICKET });
      }
      if (url.startsWith('/api/v3/views/my_planner_drafts')) {
        draftsCallCount++;
        if (draftsCallCount === 1) {
          return read1Deferred.promise;
        }
        if (draftsCallCount === 2) {
          return read2Deferred.promise;
        }
      }
      return json(404, {});
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-01" branch="SIAM" />,
      host,
    );
    await flush();

    const draftsBtn = getDraftsRefreshButton(host);

    // Overlapping click starts read 2 while mount read 1 is in flight
    draftsBtn.removeAttribute('disabled');
    dispatchClick(draftsBtn);
    await flush();

    // Later read (read 2) answers first with 2 drafts
    read2Deferred.resolve(
      json(200, { version: 2, changed: true, data: { drafts: [DRAFT_1, DRAFT_2] } }),
    );
    await flush();

    expect(host.querySelectorAll('[data-testid="draft-card"]').length).toBe(2);

    // Earlier read (read 1) answers second with 1 draft
    read1Deferred.resolve(
      json(200, { version: 1, changed: true, data: { drafts: [DRAFT_1] } }),
    );
    await flush();

    // Must still show 2 drafts
    expect(host.querySelectorAll('[data-testid="draft-card"]').length).toBe(2);
  });

  it('row 5: An earlier read fails after a later read succeeded -> No error is shown; the later state stays', async () => {
    const read1Deferred = deferred<Response>();
    const read2Deferred = deferred<Response>();
    let queueCallCount = 0;

    const testFetch = async (url: string): Promise<Response> => {
      if (url.startsWith('/api/v3/views/my_queue_ticket')) {
        queueCallCount++;
        if (queueCallCount === 1) {
          return read1Deferred.promise;
        }
        if (queueCallCount === 2) {
          return read2Deferred.promise;
        }
      }
      if (url.startsWith('/api/v3/views/my_planner_drafts')) {
        return json(200, { version: 1, changed: true, data: { drafts: [] } });
      }
      return json(404, {});
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-01" branch="SIAM" />,
      host,
    );
    await flush();

    const queueBtn = getQueueRefreshButton(host);
    queueBtn.removeAttribute('disabled');
    dispatchClick(queueBtn);
    await flush();

    // Read 2 succeeds with CALLED
    read2Deferred.resolve(json(200, { version: 2, changed: true, data: CALLED_TICKET }));
    await flush();

    expect(host.querySelector('[data-testid="ticket-status"]')?.className).toContain(
      'xc-journey-badge-called',
    );
    expect(host.querySelector('[data-testid="queue-error-detail"]')).toBeNull();

    // Read 1 fails with 500 error
    read1Deferred.resolve(json(500, { error: { code: 'serverError', message: 'Internal error' } }));
    await flush();

    // Error is ignored; CALLED state stays
    expect(host.querySelector('[data-testid="queue-error-detail"]')).toBeNull();
    expect(host.querySelector('[data-testid="ticket-status"]')?.className).toContain(
      'xc-journey-badge-called',
    );
  });

  it('row 6: A read answers -> The button is enabled again (kept behaviour after load)', async () => {
    const queueDeferred = deferred<Response>();
    const draftsDeferred = deferred<Response>();
    let queueCallCount = 0;
    let draftsCallCount = 0;

    const testFetch = async (url: string): Promise<Response> => {
      if (url.startsWith('/api/v3/views/my_queue_ticket')) {
        queueCallCount++;
        if (queueCallCount === 1) {
          return json(200, { version: 1, changed: true, data: WAITING_TICKET });
        }
        return queueDeferred.promise;
      }
      if (url.startsWith('/api/v3/views/my_planner_drafts')) {
        draftsCallCount++;
        if (draftsCallCount === 1) {
          return json(200, { version: 1, changed: true, data: { drafts: [DRAFT_1] } });
        }
        return draftsDeferred.promise;
      }
      return json(404, {});
    };

    dispose = render(
      () => <JourneyScreen fetch={testFetch} ticketId="tkt-01" branch="SIAM" />,
      host,
    );
    await flush();

    const queueBtn = getQueueRefreshButton(host);
    const draftsBtn = getDraftsRefreshButton(host);

    expect(queueBtn.disabled).toBe(false);
    expect(draftsBtn.disabled).toBe(false);

    queueBtn.click();
    draftsBtn.click();
    await flush();

    expect(queueBtn.disabled).toBe(true);
    expect(draftsBtn.disabled).toBe(true);

    queueDeferred.resolve(json(200, { version: 2, changed: true, data: WAITING_TICKET }));
    await flush();
    expect(queueBtn.disabled).toBe(false);
    expect(draftsBtn.disabled).toBe(true);

    draftsDeferred.resolve(json(200, { version: 2, changed: true, data: { drafts: [DRAFT_1] } }));
    await flush();
    expect(queueBtn.disabled).toBe(false);
    expect(draftsBtn.disabled).toBe(false);
  });
});
