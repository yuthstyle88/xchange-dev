import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Consent } from './Consent';
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

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  const u = new URL(url, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();

  if (method === 'GET' && u.pathname === '/api/v3/views/consent_texts') {
    return json(200, {
      version: 1,
      changed: true,
      data: { texts: [] },
    });
  }

  if (method === 'POST' && u.pathname === '/api/consent-texts') {
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
    () => <Consent branch={branch} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return container;
}

function setBodyText(container: HTMLElement, value: string): void {
  const textarea = container.querySelector<HTMLTextAreaElement>('.consent-publish-form textarea');
  expect(textarea, 'consent body textarea').not.toBeNull();
  textarea!.value = value;
  textarea!.dispatchEvent(new Event('input', { bubbles: true }));
  textarea!.dispatchEvent(new Event('change', { bubbles: true }));
}

function clickPublish(container: HTMLElement): void {
  const buttons = [...container.querySelectorAll<HTMLButtonElement>('.consent-publish-form button')];
  const btn = buttons.find((b) => b.textContent?.trim() === table.th.publish);
  expect(btn, 'publish button').toBeDefined();
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

describe('KW-2 (#2022): Consent main write idempotency key behaviour', () => {
  it('1. submit -> network TypeError -> submit again with the same input: 2 requests with the same key and a byte-equal body', async () => {
    writeQueue.push(
      () => {
        throw new TypeError('Failed to fetch');
      },
      () => json(200, { purpose: 'MARKETING', lang: 'th', version: 1 }),
    );

    const container = mountScreen();
    await settle();

    setBodyText(container, 'ยินยอมรับข่าวสารการตลาด');
    clickPublish(container);
    await settle();

    clickPublish(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('2. submit -> 500 -> submit again: same key', async () => {
    writeQueue.push(
      () => json(500, { error: { code: 'serverError', message: 'Internal Server Error' } }),
      () => json(200, { purpose: 'MARKETING', lang: 'th', version: 1 }),
    );

    const container = mountScreen();
    await settle();

    setBodyText(container, 'ยินยอมรับข่าวสารการตลาด');
    clickPublish(container);
    await settle();

    clickPublish(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('3. submit -> 4xx refusal -> submit again: a new key', async () => {
    writeQueue.push(
      () => json(409, { error: { code: 'outOfScope', message: 'Not allowed' } }),
      () => json(200, { purpose: 'MARKETING', lang: 'th', version: 1 }),
    );

    const container = mountScreen();
    await settle();

    setBodyText(container, 'ยินยอมรับข่าวสารการตลาด');
    clickPublish(container);
    await settle();

    clickPublish(container);
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
      () => json(200, { purpose: 'MARKETING', lang: 'th', version: 1 }),
    );

    const container = mountScreen();
    await settle();

    setBodyText(container, 'ข้อความรุ่นที่ 1');
    clickPublish(container);
    await settle();

    setBodyText(container, 'ข้อความรุ่นที่ 2 (แก้ไข)');
    clickPublish(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).not.toBe(writes[0]!.rawBody);
  });

  it('5. submit -> 2xx -> submit the same input again: a new key', async () => {
    writeQueue.push(
      () => json(200, { purpose: 'MARKETING', lang: 'th', version: 1 }),
      () => json(200, { purpose: 'MARKETING', lang: 'th', version: 2 }),
    );

    const container = mountScreen();
    await settle();

    setBodyText(container, 'ยินยอมรับข่าวสารการตลาด');
    clickPublish(container);
    await settle();

    setBodyText(container, 'ยินยอมรับข่าวสารการตลาด');
    clickPublish(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });
});
