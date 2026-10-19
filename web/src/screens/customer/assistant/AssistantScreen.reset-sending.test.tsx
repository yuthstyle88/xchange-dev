// #2712: Customer assistant — pressing "New conversation" during an in-flight send
// must immediately re-enable the input and Send button, and a stale send settling
// must not clear `sending` while a newer send is in flight.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import type { AssistantAskAnswerDto } from '../../../shared/api/generated/assistant_ask';
import type { MyAssistantConversationData } from '../../../shared/api/generated/my_assistant_conversation';
import { AssistantScreen } from './AssistantScreen';

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

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function makeAnswer(
  conversationId: string,
  customerText: string,
  assistantText: string,
): AssistantAskAnswerDto {
  return {
    conversationId,
    customerMessage: {
      id: `${conversationId}-c`,
      role: 'CUSTOMER',
      redactedText: customerText,
      createdAt: '2026-10-16T10:00:00Z',
    },
    assistantReply: {
      id: `${conversationId}-a`,
      role: 'ASSISTANT',
      redactedText: assistantText,
      createdAt: '2026-10-16T10:00:01Z',
    },
    intent: { kind: 'UNKNOWN' },
    preview: null,
  };
}

const INITIAL_CONVERSATION: MyAssistantConversationData = {
  conversation: {
    id: 'conv-initial',
    locale: 'th',
    createdAt: '2026-10-16T09:00:00Z',
  },
  messages: [
    {
      id: 'm-init-1',
      role: 'ASSISTANT',
      redactedText: 'สวัสดีค่ะ มีอะไรให้ช่วยไหมคะ',
      createdAt: '2026-10-16T09:00:01Z',
    },
  ],
};

describe('AssistantScreen resetConversation sending state (#2712)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/assistant');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host?.remove();
    window.history.replaceState({}, '', '/');
    setLang('th');
    setAudience('staff');
    vi.restoreAllMocks();
  });

  async function flush(): Promise<void> {
    for (let i = 0; i < 6; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  function typeInput(text: string): HTMLInputElement {
    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    expect(input, 'assistant input').not.toBeNull();
    input.value = text;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    return input;
  }

  function typeAndSubmit(text: string): void {
    typeInput(text);
    const form = host.querySelector('.assistant-form') as HTMLFormElement;
    expect(form, 'assistant form').not.toBeNull();
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  }

  it('row 1: A send is held; press New conversation -> .assistant-input and .assistant-send-button are enabled at once and button reads t("send") while old POST is still pending', async () => {
    const heldSend = deferred<Response>();

    const mockFetch = vi.fn((input: string, init?: RequestInit) => {
      const url = new URL(input, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname === '/api/v3/views/my_assistant_conversation') {
        return Promise.resolve(
          jsonResponse({ version: 1, changed: true, data: INITIAL_CONVERSATION }),
        );
      }
      if (method === 'POST' && url.pathname === '/api/app/assistant/messages') {
        return heldSend.promise;
      }
      return Promise.resolve(jsonResponse({}, 404));
    });

    dispose = render(
      () => <AssistantScreen fetch={mockFetch} initialConversationId="conv-initial" />,
      host,
    );

    await vi.waitFor(() => {
      expect(host.querySelector('.assistant-messages')).not.toBeNull();
    });

    typeAndSubmit('คำถามค้างอยู่');
    await flush();

    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    const sendBtn = host.querySelector('.assistant-send-button') as HTMLButtonElement;
    expect(input.disabled).toBe(true);
    expect(sendBtn.disabled).toBe(true);
    expect(sendBtn.textContent?.trim()).toBe('กำลังส่ง…');

    // Press "New conversation" while the send is still pending
    const resetBtn = host.querySelector('.assistant-reset-button') as HTMLButtonElement;
    expect(resetBtn).not.toBeNull();
    resetBtn.click();
    await flush();

    expect(input.disabled).toBe(false);
    expect(sendBtn.textContent?.trim()).toBe('ส่ง');

    typeInput('ข้อความใหม่');
    expect(sendBtn.disabled).toBe(false);
  });

  it('row 2: After 1, send a new message; the old POST settles before the new one -> input stays disabled until the new POST settles', async () => {
    let postCall = 0;
    const firstSend = deferred<Response>();
    const secondSend = deferred<Response>();

    const mockFetch = vi.fn((input: string, init?: RequestInit) => {
      const url = new URL(input, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname === '/api/v3/views/my_assistant_conversation') {
        return Promise.resolve(
          jsonResponse({ version: 1, changed: true, data: INITIAL_CONVERSATION }),
        );
      }
      if (method === 'POST' && url.pathname === '/api/app/assistant/messages') {
        postCall++;
        return postCall === 1 ? firstSend.promise : secondSend.promise;
      }
      return Promise.resolve(jsonResponse({}, 404));
    });

    dispose = render(
      () => <AssistantScreen fetch={mockFetch} initialConversationId="conv-initial" />,
      host,
    );

    await vi.waitFor(() => {
      expect(host.querySelector('.assistant-messages')).not.toBeNull();
    });

    // 1. Start first send (held) and press New conversation
    typeAndSubmit('คำถามแรก');
    await flush();

    const resetBtn = host.querySelector('.assistant-reset-button') as HTMLButtonElement;
    resetBtn.click();
    await flush();

    // 2. Send a new message in the new conversation (also held)
    typeAndSubmit('คำถามที่สอง');
    await flush();
    expect(postCall).toBe(2);

    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    const sendBtn = host.querySelector('.assistant-send-button') as HTMLButtonElement;
    expect(input.disabled).toBe(true);
    expect(sendBtn.disabled).toBe(true);
    expect(sendBtn.textContent?.trim()).toBe('กำลังส่ง…');

    // 3. The old POST settles first -> input and send button must stay disabled!
    firstSend.resolve(jsonResponse(makeAnswer('conv-initial', 'คำถามแรก', 'คำตอบแรก')));
    await flush();

    expect(input.disabled).toBe(true);
    expect(sendBtn.disabled).toBe(true);
    expect(sendBtn.textContent?.trim()).toBe('กำลังส่ง…');

    // 4. Now the new POST settles -> input re-enables and button reads t('send')
    secondSend.resolve(jsonResponse(makeAnswer('conv-second', 'คำถามที่สอง', 'คำตอบที่สอง')));
    await flush();

    expect(input.disabled).toBe(false);
    expect(sendBtn.textContent?.trim()).toBe('ส่ง');
    expect(host.textContent).toContain('คำตอบที่สอง');
  });

  it('row 3: Held send, New conversation, the old send answers or fails -> old reply or error is ignored and URL stays clean', async () => {
    const heldSend = deferred<Response>();

    const mockFetch = vi.fn((input: string, init?: RequestInit) => {
      const url = new URL(input, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname === '/api/v3/views/my_assistant_conversation') {
        return Promise.resolve(
          jsonResponse({ version: 1, changed: true, data: INITIAL_CONVERSATION }),
        );
      }
      if (method === 'POST' && url.pathname === '/api/app/assistant/messages') {
        return heldSend.promise;
      }
      return Promise.resolve(jsonResponse({}, 404));
    });

    window.history.replaceState({}, '', '/app/assistant?conversation=conv-initial');
    dispose = render(
      () => <AssistantScreen fetch={mockFetch} initialConversationId="conv-initial" />,
      host,
    );

    await vi.waitFor(() => {
      expect(host.querySelector('.assistant-messages')).not.toBeNull();
    });

    typeAndSubmit('คำถามที่จะถูกยกเลิก');
    await flush();

    const resetBtn = host.querySelector('.assistant-reset-button') as HTMLButtonElement;
    resetBtn.click();
    await flush();

    heldSend.reject(new TypeError('Failed to fetch'));
    await flush();

    expect(host.querySelector('.assistant-messages')).toBeNull();
    expect(host.querySelector('[role="alert"].assistant-send-error')).toBeNull();
    expect(new URL(window.location.href).searchParams.get('conversation')).toBeNull();
  });

  it('row 4: A normal send with no reset -> input is disabled while sending and enabled after the reply', async () => {
    const heldSend = deferred<Response>();

    const mockFetch = vi.fn((input: string, init?: RequestInit) => {
      const url = new URL(input, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'POST' && url.pathname === '/api/app/assistant/messages') {
        return heldSend.promise;
      }
      return Promise.resolve(jsonResponse({}, 404));
    });

    dispose = render(() => <AssistantScreen fetch={mockFetch} />, host);
    await flush();

    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    const sendBtn = host.querySelector('.assistant-send-button') as HTMLButtonElement;
    expect(input.disabled).toBe(false);

    typeAndSubmit('สวัสดี');
    await flush();

    expect(input.disabled).toBe(true);
    expect(sendBtn.disabled).toBe(true);
    expect(sendBtn.textContent?.trim()).toBe('กำลังส่ง…');

    heldSend.resolve(jsonResponse(makeAnswer('conv-1', 'สวัสดี', 'ยินดีให้บริการค่ะ')));
    await flush();

    expect(input.disabled).toBe(false);
    expect(sendBtn.textContent?.trim()).toBe('ส่ง');
    expect(host.textContent).toContain('ยินดีให้บริการค่ะ');
  });

  it('edge: Prompt-suggestion click after reset during held send sends immediately and stays disabled until new send settles', async () => {
    let postCall = 0;
    const firstSend = deferred<Response>();
    const promptSend = deferred<Response>();

    const mockFetch = vi.fn((input: string, init?: RequestInit) => {
      const url = new URL(input, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname === '/api/v3/views/my_assistant_conversation') {
        return Promise.resolve(
          jsonResponse({ version: 1, changed: true, data: INITIAL_CONVERSATION }),
        );
      }
      if (method === 'POST' && url.pathname === '/api/app/assistant/messages') {
        postCall++;
        return postCall === 1 ? firstSend.promise : promptSend.promise;
      }
      return Promise.resolve(jsonResponse({}, 404));
    });

    dispose = render(
      () => <AssistantScreen fetch={mockFetch} initialConversationId="conv-initial" />,
      host,
    );

    await vi.waitFor(() => {
      expect(host.querySelector('.assistant-messages')).not.toBeNull();
    });

    typeAndSubmit('คำถามแรก');
    await flush();

    const resetBtn = host.querySelector('.assistant-reset-button') as HTMLButtonElement;
    resetBtn.click();
    await flush();

    const promptBtn = host.querySelector('.assistant-prompt-button') as HTMLButtonElement;
    expect(promptBtn).not.toBeNull();
    promptBtn.click();
    await flush();

    expect(postCall).toBe(2);
    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    expect(input.disabled).toBe(true);

    firstSend.resolve(jsonResponse(makeAnswer('conv-initial', 'คำถามแรก', 'คำตอบแรก')));
    await flush();
    expect(input.disabled).toBe(true);

    promptSend.resolve(jsonResponse(makeAnswer('conv-prompt', 'เรทวันนี้เท่าไหร่?', 'เรท USD 35.00')));
    await flush();
    expect(input.disabled).toBe(false);
    expect(host.textContent).toContain('เรท 35.00 USD');
  });
});
