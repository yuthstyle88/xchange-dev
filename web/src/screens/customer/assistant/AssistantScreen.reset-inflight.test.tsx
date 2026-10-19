// #2535: Customer assistant — pressing "New conversation" while a reply is pending
// must ignore the in-flight send's completion or failure so the old conversation never comes back.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import {
  makeAssistantAskAnswerDto,
  makeAssistantConversationDto,
  makeAssistantMessageDto,
  makeMyAssistantConversationData,
} from '../../../shared/testing/fixtures';
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
) {
  return makeAssistantAskAnswerDto({
    conversationId,
    customerMessage: makeAssistantMessageDto({
      id: `${conversationId}-c`,
      role: 'CUSTOMER',
      redactedText: customerText,
      createdAt: '2026-10-16T10:00:00Z',
    }),
    assistantReply: makeAssistantMessageDto({
      id: `${conversationId}-a`,
      role: 'ASSISTANT',
      redactedText: assistantText,
      createdAt: '2026-10-16T10:00:01Z',
    }),
  });
}

const INITIAL_CONVERSATION = makeMyAssistantConversationData({
  conversation: makeAssistantConversationDto({
    id: 'conv-initial',
    locale: 'th',
    createdAt: '2026-10-16T09:00:00Z',
  }),
  messages: [
    makeAssistantMessageDto({
      id: 'm-init-1',
      role: 'ASSISTANT',
      redactedText: 'สวัสดีค่ะ มีอะไรให้ช่วยไหมคะ',
      createdAt: '2026-10-16T09:00:01Z',
    }),
  ],
});

describe('AssistantScreen resetConversation while send is in flight (#2535)', () => {
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

  function typeAndSubmit(text: string): void {
    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    expect(input, 'assistant input').not.toBeNull();
    input.value = text;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const form = host.querySelector('.assistant-form') as HTMLFormElement;
    expect(form, 'assistant form').not.toBeNull();
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  }

  it('row 1: Send held, press New conversation, send answers -> messages stay empty, URL has no conversation param, conversationId null', async () => {
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

    // Start a send and hold the response
    typeAndSubmit('คำถามค้างอยู่');
    await flush();

    // Press "New conversation" while the send is still in flight
    const resetBtn = host.querySelector('.assistant-reset-button') as HTMLButtonElement;
    expect(resetBtn).not.toBeNull();
    resetBtn.click();
    await flush();

    expect(host.querySelector('.assistant-messages')).toBeNull();
    expect(new URL(window.location.href).searchParams.get('conversation')).toBeNull();

    // Now the in-flight send resolves with the old conversation
    heldSend.resolve(
      jsonResponse(makeAnswer('conv-initial', 'คำถามค้างอยู่', 'คำตอบของบทสนทนาเก่า')),
    );
    await flush();

    // Messages stay empty, URL has no conversation param, and conversationId remains null
    expect(host.querySelector('.assistant-messages')).toBeNull();
    expect(host.textContent).not.toContain('คำถามค้างอยู่');
    expect(host.textContent).not.toContain('คำตอบของบทสนทนาเก่า');
    expect(new URL(window.location.href).searchParams.get('conversation')).toBeNull();
  });

  it('row 2: Send held, press New conversation, send fails -> input stays empty, no send error shown', async () => {
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

    // Type a question and submit; input clears while sending
    typeAndSubmit('คำถามที่จะล้มเหลว');
    await flush();
    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    expect(input.value).toBe('');

    // Press "New conversation" while the send is still in flight
    const resetBtn = host.querySelector('.assistant-reset-button') as HTMLButtonElement;
    expect(resetBtn).not.toBeNull();
    resetBtn.click();
    await flush();

    // Now the in-flight send fails
    heldSend.reject(new TypeError('Failed to fetch'));
    await flush();

    // Input stays empty and no send error is shown in the new conversation
    expect(input.value).toBe('');
    expect(host.querySelector('[role="alert"].assistant-send-error')).toBeNull();
  });

  it('row 3: Send without reset appends answer and updates URL, and a subsequent reset drops an in-flight follow-up', async () => {
    let postCall = 0;
    const secondSend = deferred<Response>();

    const mockFetch = vi.fn((input: string, init?: RequestInit) => {
      const url = new URL(input, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'POST' && url.pathname === '/api/app/assistant/messages') {
        postCall++;
        if (postCall === 1) {
          return Promise.resolve(
            jsonResponse(makeAnswer('conv-first', 'เรท USD วันนี้', 'USD รับซื้อ 35.00')),
          );
        }
        return secondSend.promise;
      }
      return Promise.resolve(jsonResponse({}, 404));
    });

    dispose = render(() => <AssistantScreen fetch={mockFetch} />, host);
    await flush();

    // First send without reset: appends answer and updates URL
    typeAndSubmit('เรท USD วันนี้');
    await vi.waitFor(() => {
      expect(host.querySelector('.assistant-messages')).not.toBeNull();
    });
    expect(host.textContent).toContain('เรท USD วันนี้');
    expect(host.textContent).toContain('USD รับซื้อ 35.00');
    expect(new URL(window.location.href).searchParams.get('conversation')).toBe('conv-first');

    // Second send held, then reset pressed before reply arrives
    typeAndSubmit('ถามต่อก่อนรีเซ็ต');
    await flush();

    const resetBtn = host.querySelector('.assistant-reset-button') as HTMLButtonElement;
    resetBtn.click();
    await flush();

    secondSend.resolve(
      jsonResponse(makeAnswer('conv-first', 'ถามต่อก่อนรีเซ็ต', 'คำตอบที่ไม่ควรกลับมา')),
    );
    await flush();

    expect(host.querySelector('.assistant-messages')).toBeNull();
    expect(new URL(window.location.href).searchParams.get('conversation')).toBeNull();
  });

  it('edge: After the reset, a new send works normally and its answer is shown without the old conversation', async () => {
    let postCall = 0;
    const staleSend = deferred<Response>();

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
        if (postCall === 1) {
          return staleSend.promise;
        }
        return Promise.resolve(
          jsonResponse(makeAnswer('conv-fresh', 'คำถามใหม่หลังรีเซ็ต', 'คำตอบใหม่สุด')),
        );
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

    // 1. Start old send and reset while it is in flight
    typeAndSubmit('คำถามเก่าก่อนรีเซ็ต');
    await flush();

    const resetBtn = host.querySelector('.assistant-reset-button') as HTMLButtonElement;
    resetBtn.click();
    await flush();

    // Old send finishes after reset
    staleSend.resolve(
      jsonResponse(makeAnswer('conv-initial', 'คำถามเก่าก่อนรีเซ็ต', 'คำตอบเก่าที่ไม่ควรแสดง')),
    );
    await flush();

    // 2. Now send a new message in the fresh conversation
    typeAndSubmit('คำถามใหม่หลังรีเซ็ต');
    await vi.waitFor(() => {
      expect(host.querySelector('.assistant-messages')).not.toBeNull();
    });

    expect(host.textContent).toContain('คำถามใหม่หลังรีเซ็ต');
    expect(host.textContent).toContain('คำตอบใหม่สุด');
    expect(host.textContent).not.toContain('คำถามเก่าก่อนรีเซ็ต');
    expect(host.textContent).not.toContain('คำตอบเก่าที่ไม่ควรแสดง');
    expect(host.querySelectorAll('.message-wrapper')).toHaveLength(2);
    expect(new URL(window.location.href).searchParams.get('conversation')).toBe('conv-fresh');
  });
});
