import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
import { Members } from './Members';

beforeAll(
  () =>
    warmModules(
      () => import('./Members'),
      () => import('./i18n'),
      () => import('../../../shared/api/client'),
      () => import('../../../shared/i18n'),
      () => import('../../../shared/i18n/worded-error'),
    ),
  60_000,
);

interface CapturedWrite {
  method: string;
  pathname: string;
  idempotencyKey: string;
  rawBody: string;
  parsedBody: { name: string; rule: Record<string, unknown> };
}

function makeProps(): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => realSetTimeout(r, 0));
  }
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Admin Members: refuse non-integer recency_days and frequency in segment rules (#2528)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let defineCalls: CapturedWrite[];

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);
    defineCalls = [];

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl =
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const idempotencyKey = headers.get('idempotency-key') ?? '';
      const rawBody = typeof init?.body === 'string' ? init.body : '';

      if (method === 'POST' && u.pathname === '/api/segments') {
        const entry: CapturedWrite = {
          method,
          pathname: u.pathname,
          idempotencyKey,
          rawBody,
          parsedBody: JSON.parse(rawBody) as { name: string; rule: Record<string, unknown> },
        };
        defineCalls.push(entry);
        return jsonResponse(200, { id: `seg-${defineCalls.length}`, version: 1 });
      }

      return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
    });
  });

  afterEach(async () => {
    await settle();
    dispose?.();
    dispose = undefined;
    root.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function mountMembers() {
    dispose = render(() => <Members {...makeProps()} />, root);
  }

  function setInput(selector: string, val: string) {
    const input = root.querySelector<HTMLInputElement>(selector);
    expect(input, selector).not.toBeNull();
    // Allow arbitrary raw strings (such as ' 45 ' or '7days') without JSDOM number sanitization
    input!.type = 'text';
    input!.value = val;
    input!.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function defineButton(): HTMLButtonElement {
    const btn = root.querySelector<HTMLButtonElement>('button[data-action="define"]');
    expect(btn).not.toBeNull();
    return btn!;
  }

  function errorBannerText(): string | null {
    const alert = root.querySelector('.xc-campaigns-error[role="alert"]');
    return alert ? (alert.textContent ?? '').trim() : null;
  }

  it('row 1: Recency 1.5, Define -> No POST /api/segments; the validation sentence is shown', async () => {
    mountMembers();
    setInput('#segment-name', 'VIP Segment');
    setInput('#segment-recency-days', '1.5');

    defineButton().click();
    await settle();

    expect(defineCalls).toHaveLength(0);
    expect(errorBannerText()).toBe(table.th['error.validationFailed']);
  });

  it('row 2: Recency 1e3 -> No POST /api/segments; the validation sentence is shown', async () => {
    mountMembers();
    setInput('#segment-name', 'VIP Segment');
    setInput('#segment-recency-days', '1e3');

    defineButton().click();
    await settle();

    expect(defineCalls).toHaveLength(0);
    expect(errorBannerText()).toBe(table.th['error.validationFailed']);
  });

  it('row 3: Frequency 2.5 (recency 30) -> No POST /api/segments; the validation sentence is shown', async () => {
    mountMembers();
    setInput('#segment-name', 'VIP Segment');
    setInput('#segment-recency-days', '30');
    setInput('#segment-frequency', '2.5');

    defineButton().click();
    await settle();

    expect(defineCalls).toHaveLength(0);
    expect(errorBannerText()).toBe(table.th['error.validationFailed']);
  });

  it('row 4: Recency "7days" is refused, whereas Recency " 45 " (spaces) sends rule.recency_days: 45', async () => {
    mountMembers();
    setInput('#segment-name', 'VIP Segment');
    setInput('#segment-recency-days', '7days');

    defineButton().click();
    await settle();
    expect(defineCalls).toHaveLength(0);
    expect(errorBannerText()).toBe(table.th['error.validationFailed']);

    setInput('#segment-recency-days', ' 45 ');
    defineButton().click();
    await settle();

    expect(defineCalls).toHaveLength(1);
    expect(defineCalls[0]!.parsedBody.rule).toEqual({ recency_days: 45 });
    expect(errorBannerText()).toBeNull();
  });

  it('row 5: Frequency "1e2" with empty recency is refused, whereas empty recency and empty frequency sends a rule without recency_days and frequency', async () => {
    mountMembers();
    setInput('#segment-name', 'All Customers');
    setInput('#segment-recency-days', '');
    setInput('#segment-frequency', '1e2');

    defineButton().click();
    await settle();
    expect(defineCalls).toHaveLength(0);
    expect(errorBannerText()).toBe(table.th['error.validationFailed']);

    setInput('#segment-frequency', '');
    defineButton().click();
    await settle();

    expect(defineCalls).toHaveLength(1);
    expect(defineCalls[0]!.parsedBody.rule).toEqual({});
    expect('recency_days' in defineCalls[0]!.parsedBody.rule).toBe(false);
    expect('frequency' in defineCalls[0]!.parsedBody.rule).toBe(false);
  });

  it('row 6: Define flow and idempotency key (kept) after refusing a non-integer rule field', async () => {
    mountMembers();
    setInput('#segment-name', 'Repeat Buyers');
    setInput('#segment-recency-days', '30');
    setInput('#segment-frequency', '3.5');

    defineButton().click();
    await settle();
    expect(defineCalls).toHaveLength(0);

    setInput('#segment-frequency', '3');
    defineButton().click();
    await settle();

    expect(defineCalls).toHaveLength(1);
    expect(defineCalls[0]!.idempotencyKey).toBeTruthy();
    expect(defineCalls[0]!.parsedBody).toEqual({
      name: 'Repeat Buyers',
      rule: { recency_days: 30, frequency: 3 },
    });
  });

  it('edge case: -5 is refused, and after a refusal fixing the field and pressing Define sends the request (defining flag cleared)', async () => {
    mountMembers();
    setInput('#segment-name', 'Active Members');
    setInput('#segment-recency-days', '-5');

    defineButton().click();
    await settle();

    expect(defineCalls).toHaveLength(0);
    expect(errorBannerText()).toBe(table.th['error.validationFailed']);
    expect(defineButton().disabled).toBe(false);

    setInput('#segment-recency-days', '30');
    setInput('#segment-frequency', '-5');

    defineButton().click();
    await settle();

    expect(defineCalls).toHaveLength(0);
    expect(errorBannerText()).toBe(table.th['error.validationFailed']);
    expect(defineButton().disabled).toBe(false);

    setInput('#segment-frequency', '4');
    defineButton().click();
    await settle();

    expect(defineCalls).toHaveLength(1);
    expect(defineCalls[0]!.parsedBody.rule).toEqual({ recency_days: 30, frequency: 4 });
    expect(errorBannerText()).toBeNull();
  });
});
