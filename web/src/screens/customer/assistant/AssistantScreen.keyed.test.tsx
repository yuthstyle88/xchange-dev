// #2023 KW-4 Phase 1: AssistantScreen idempotency-key retry behaviour (§9 rows 1–5) through HTTP only.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import type { AssistantAskAnswerDto } from '../../../shared/api/generated/assistant_ask';
import { setLang } from '../../../shared/i18n';
import { AssistantScreen } from './AssistantScreen';

interface WriteCall {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepAnswer =
  | { kind: 'network' }
  | { kind: 'http'; status: number; body: unknown };

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const ASK_ANSWER: AssistantAskAnswerDto = {
  conversationId: 'conv-1',
  customerMessage: {
    id: 'msg-1',
    role: 'CUSTOMER',
    redactedText: 'เรท USD วันนี้',
    createdAt: '2026-10-15T08:00:00Z',
  },
  assistantReply: {
    id: 'msg-2',
    role: 'ASSISTANT',
    redactedText: 'เรท USD วันนี้คือ 35.10 บาท',
    createdAt: '2026-10-15T08:00:01Z',
  },
};

function makeHarness(steps: StepAnswer[]) {
  const writes: WriteCall[] = [];
  let stepIdx = 0;

  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');

    if (method === 'POST' && u.pathname === '/api/app/assistant/messages') {
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : '';
      writes.push({
        method,
        path: u.pathname,
        key: headers.get('idempotency-key'),
        body,
      });
      const step = steps[stepIdx++] ?? { kind: 'http', status: 200, body: ASK_ANSWER };
      if (step.kind === 'network') {
        throw new TypeError('Failed to fetch');
      }
      return jsonResponse(step.status, step.body);
    }

    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  return {
    api: new ApiClient(fetchFn),
    writes,
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('AssistantScreen.tsx — §9 idempotency-key retry behaviour (#2023 KW-4)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mountScreen(api: ApiClient): void {
    dispose = render(() => <AssistantScreen api={api} branch="MAIN" />, host);
  }

  function sendText(text: string): void {
    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    expect(input).not.toBeNull();
    input.value = text;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const sendBtn = host.querySelector('.assistant-send-button') as HTMLButtonElement;
    expect(sendBtn).not.toBeNull();
    sendBtn.click();
  }

  it('row 1: submit → network TypeError → submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    const { api, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: ASK_ANSWER },
    ]);

    mountScreen(api);
    sendText('เรท USD วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    sendText('เรท USD วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit → 500 → submit again sends 2 requests with the same key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'Server error' } } },
      { kind: 'http', status: 200, body: ASK_ANSWER },
    ]);

    mountScreen(api);
    sendText('เรท USD วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    sendText('เรท USD วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit → 4xx refusal → submit again sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 400, body: { error: { code: 'validationFailed', message: 'Invalid question' } } },
      { kind: 'http', status: 200, body: ASK_ANSWER },
    ]);

    mountScreen(api);
    sendText('เรท USD วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    sendText('เรท USD วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit → network error → edit the input → submit sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: ASK_ANSWER },
    ]);

    mountScreen(api);
    sendText('เรท USD วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    sendText('เรท EUR วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 5: submit → 2xx → submit the same input again sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 200, body: ASK_ANSWER },
      { kind: 'http', status: 200, body: ASK_ANSWER },
    ]);

    mountScreen(api);
    sendText('เรท USD วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    sendText('เรท USD วันนี้');
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
