import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Approvals } from './Approvals';
import * as table from './i18n';

interface WriteCall {
  url: string;
  method: string;
  key: string | null;
  rawBody: string;
}

type WriteResponder = () => Promise<Response> | Response;

let writes: WriteCall[] = [];
let writeQueue: WriteResponder[] = [];
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const SEED_REQUEST = {
  id: 'r1',
  type: 'RATE_PUBLICATION',
  branchId: 'MAIN',
  objectId: 'draft-of-r1',
  objectVersionHash: 'hash-r1',
  policyVersion: 'pol-1',
  proposedPayload: {
    draft_id: 'draft-of-r1',
    policy_version_id: 'pol-1',
    reference_line_ids: ['line-1'],
    is_in_bound: false,
    out_of_bound: [{ currency: 'USD', side: 'SELL' }],
  },
  requestedBy: 'employee-RATES',
  requestedAt: '2026-10-09T03:00:00+00:00',
  status: 'PENDING',
  decision: null,
};

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  const u = new URL(url, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();

  if (method === 'GET' && u.pathname === '/api/v3/views/approval_requests') {
    return json(200, {
      version: 1,
      changed: true,
      data: { requests: [structuredClone(SEED_REQUEST)] },
    });
  }

  if (method === 'POST' && u.pathname === '/api/approvals/r1/decide') {
    const headers = new Headers(init?.headers);
    writes.push({
      url: u.pathname,
      method,
      key: headers.get('idempotency-key'),
      rawBody: typeof init?.body === 'string' ? init.body : '',
    });
    const next = writeQueue.shift();
    if (!next) {
      throw new Error('unexpected write with empty writeQueue');
    }
    return await next();
  }

  return json(500, { error: { code: 'serverError', message: `unexpected ${method} ${u.pathname}` } });
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function mountScreen(): HTMLElement {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-09', to: '2026-10-09' });
  const [shown] = createSignal(true);
  const container = document.createElement('div');
  document.body.appendChild(container);
  const dispose = render(
    () => <Approvals branch={branch} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return container;
}

function setReason(container: HTMLElement, value: string): void {
  const input = container.querySelector<HTMLInputElement>('[data-request="r1"] input[type="text"]');
  expect(input, 'reason input in [data-request="r1"]').not.toBeNull();
  input!.value = value;
  input!.dispatchEvent(new Event('input', { bubbles: true }));
  input!.dispatchEvent(new Event('change', { bubbles: true }));
}

function clickApprove(container: HTMLElement): void {
  const buttons = [...container.querySelectorAll<HTMLButtonElement>('[data-request="r1"] button')];
  const btn = buttons.find((b) => b.textContent?.trim() === table.th.approve);
  expect(btn, 'approve button in [data-request="r1"]').toBeDefined();
  btn!.click();
}

beforeEach(() => {
  writes = [];
  writeQueue = [];
  document.body.innerHTML = '';
  setLang('th');
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
});

afterEach(async () => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  await settle();
});

describe('KW-2 (#2022): Approvals main write idempotency key behaviour', () => {
  it('1. submit -> network TypeError -> submit again with the same input: 2 requests with the same key and a byte-equal body', async () => {
    writeQueue.push(
      () => {
        throw new TypeError('Failed to fetch');
      },
      () => json(200, { status: 'APPROVED' }),
    );

    const container = mountScreen();
    await settle();

    setReason(container, 'checked against BoT');
    clickApprove(container);
    await settle();

    clickApprove(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('2. submit -> 500 -> submit again: same key', async () => {
    writeQueue.push(
      () => json(500, { error: { code: 'serverError', message: 'Internal Server Error' } }),
      () => json(200, { status: 'APPROVED' }),
    );

    const container = mountScreen();
    await settle();

    setReason(container, 'checked against BoT');
    clickApprove(container);
    await settle();

    clickApprove(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('3. submit -> 4xx refusal -> submit again: a new key', async () => {
    writeQueue.push(
      () => json(409, { error: { code: 'outOfScope', message: 'Proposer may not decide' } }),
      () => json(200, { status: 'APPROVED' }),
    );

    const container = mountScreen();
    await settle();

    setReason(container, 'checked against BoT');
    clickApprove(container);
    await settle();

    clickApprove(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('4. submit -> network error -> edit the input -> submit: a new key', async () => {
    writeQueue.push(
      () => {
        throw new TypeError('Failed to fetch');
      },
      () => json(200, { status: 'APPROVED' }),
    );

    const container = mountScreen();
    await settle();

    setReason(container, 'first reason');
    clickApprove(container);
    await settle();

    setReason(container, 'edited reason');
    clickApprove(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).not.toBe(writes[0]!.rawBody);
  });

  it('5. submit -> 2xx -> submit the same input again: a new key', async () => {
    writeQueue.push(
      () => json(200, { status: 'APPROVED' }),
      () => json(200, { status: 'APPROVED' }),
    );

    const container = mountScreen();
    await settle();

    setReason(container, 'checked against BoT');
    clickApprove(container);
    await settle();

    setReason(container, 'checked against BoT');
    clickApprove(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });
});
