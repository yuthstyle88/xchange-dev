import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { AssistantScreen } from './AssistantScreen';
import * as table from './i18n';
import type {
  AssistantAskAnswerDto,
  AssistantAskInput,
} from '../../../shared/api/generated/assistant_ask';
import type {
  MyAssistantConversationData,
} from '../../../shared/api/generated/my_assistant_conversation';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;
const EN = table.en as unknown as Texts;
const ZH = table.zh as unknown as Texts;

let fs: NodeFs;
let dir = '';
let dispose: (() => void) | undefined;

beforeAll(async () => {
  const fsName = 'node:fs';
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
});

function mount(props: Parameters<typeof AssistantScreen>[0] = {}) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <AssistantScreen {...props} />, host);
  return host;
}

function flush() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  if (typeof window !== 'undefined') {
    if (window.localStorage) {
      window.localStorage.clear();
    }
    if (window.history) {
      window.history.replaceState({}, '', '/');
    }
  }
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  if (typeof window !== 'undefined' && window.history) {
    window.history.replaceState({}, '', '/');
  }
  setLang('th');
  setAudience('staff');
  vi.restoreAllMocks();
});

describe('APP-14 C12 Assistant Screen (AssistantScreen)', () => {
  it('1. screen is named "XCHANGE Assistant", never presented as AI, and never asks for credentials or documents (P11)', () => {
    const host = mount();
    const title = host.querySelector('h1.assistant-title');
    expect(title?.textContent).toBe(TH.title);

    // Switch to English and check name
    setLang('en');
    expect(title?.textContent).toBe('XCHANGE Assistant');

    // Never presented as AI (P11: deterministic parser, never presented as AI)
    const pageText = host.textContent?.toLowerCase() ?? '';
    expect(pageText).not.toContain(' artificial intelligence ');
    expect(pageText).not.toContain(' gpt ');
    expect(pageText).not.toContain(' chatgpt ');
    expect(pageText).not.toContain(' llm ');
    expect(pageText).not.toContain(' bot ');

    // Disclaimer states that it will never ask for credentials or documents
    const disclaimer = host.querySelector('.assistant-disclaimer-banner');
    expect(disclaimer?.textContent).toBe(EN.disclaimer);
    expect(disclaimer?.textContent).toContain('never ask for your passwords, card details, or identity documents');
  });

  it('2. loading state: shows loading indicator while conversation is loading', async () => {
    let resolveView: (val: Response) => void;
    const pendingPromise = new Promise<Response>((resolve) => {
      resolveView = resolve;
    });

    const mockFetch = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/api/v3/views/my_assistant_conversation')) {
        return pendingPromise;
      }
      return Promise.reject(new Error(`Unhandled: ${url}`));
    });

    const host = mount({ fetch: mockFetch, initialConversationId: 'conv-123' });
    await flush();

    const loadingElem = host.querySelector('[role="status"]');
    expect(loadingElem).not.toBeNull();
    expect(loadingElem?.textContent).toBe(TH.loading);

    // Resolve view
    const viewData: MyAssistantConversationData = {
      conversation: {
        id: 'conv-123',
        locale: 'th',
        createdAt: '2026-10-13T03:00:00Z',
      },
      messages: [
        {
          id: 'msg-1',
          role: 'CUSTOMER',
          redactedText: 'สวัสดี',
          createdAt: '2026-10-13T03:00:01Z',
        },
      ],
    };

    resolveView!(
      new Response(JSON.stringify({ version: 1, changed: true, data: viewData }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );

    await flush();
    await flush();

    expect(host.querySelector('[role="status"]')).toBeNull();
    expect(host.querySelector('.message-bubble')?.textContent).toContain('สวัสดี');
  });

  it('3. empty state: shows empty message and prompt buttons when no messages exist', async () => {
    const host = mount();
    await flush();

    const emptyElem = host.querySelector('.assistant-empty');
    expect(emptyElem).not.toBeNull();
    expect(emptyElem?.textContent).toContain(TH.empty);

    const prompts = host.querySelectorAll('.assistant-prompt-button');
    expect(prompts.length).toBe(3);
    expect(prompts[0]?.textContent).toBe(TH.promptRates);
    expect(prompts[1]?.textContent).toBe(TH.promptBranches);
    expect(prompts[2]?.textContent).toBe(TH.promptQueue);
  });

  it('4. error and retry states: shows error on fetch failure and retries on button click', async () => {
    let callCount = 0;
    const mockFetch = vi.fn().mockImplementation((_url: string) => {
      callCount++;
      if (callCount === 1) {
        return Promise.resolve(
          new Response(JSON.stringify({ error: { code: 'serverError', message: 'DB connection failure' } }), {
            status: 500,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }
      const data: MyAssistantConversationData = {
        conversation: { id: 'conv-retry', locale: 'th', createdAt: '2026-10-13T03:00:00Z' },
        messages: [
          {
            id: 'm1',
            role: 'ASSISTANT',
            redactedText: 'ยินดีต้อนรับครับ',
            createdAt: '2026-10-13T03:00:01Z',
          },
        ],
      };
      return Promise.resolve(
        new Response(JSON.stringify({ version: 1, changed: true, data }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      );
    });

    const host = mount({ fetch: mockFetch, initialConversationId: 'conv-retry' });
    await flush();
    await flush();

    // Error is shown
    const errorAlert = host.querySelector('[role="alert"].assistant-error');
    expect(errorAlert).not.toBeNull();
    const retryBtn = host.querySelector('.assistant-retry-button') as HTMLButtonElement;
    expect(retryBtn).not.toBeNull();
    expect(retryBtn.textContent).toBe(TH.retry);

    // Click retry
    retryBtn.click();
    await flush();
    await flush();

    // After retry, error is gone and message is rendered
    expect(host.querySelector('[role="alert"].assistant-error')).toBeNull();
    expect(host.querySelector('.message-bubble')?.textContent).toContain('ยินดีต้อนรับครับ');
  });

  it('5. language switch: reflects TH, EN, and 中文, and changes UI texts across locales', async () => {
    const host = mount();
    await flush();

    // Default Thai
    expect(host.querySelector('h1.assistant-title')?.textContent).toBe(TH.title);
    expect(host.querySelector('.assistant-subtitle')?.textContent).toBe(TH.subtitle);
    expect(host.querySelector('.assistant-empty')?.textContent).toContain(TH.empty);
    expect(host.querySelector('.assistant-send-button')?.textContent).toBe(TH.send);

    // Switch to English through shared i18n setter
    setLang('en');
    await flush();
    expect(host.querySelector('h1.assistant-title')?.textContent).toBe(EN.title);
    expect(host.querySelector('.assistant-subtitle')?.textContent).toBe(EN.subtitle);
    expect(host.querySelector('.assistant-empty')?.textContent).toContain(EN.empty);
    expect(host.querySelector('.assistant-send-button')?.textContent).toBe(EN.send);

    // Switch to Chinese
    setLang('zh');
    await flush();
    expect(host.querySelector('h1.assistant-title')?.textContent).toBe(ZH.title);
    expect(host.querySelector('.assistant-subtitle')?.textContent).toBe(ZH.subtitle);
    expect(host.querySelector('.assistant-empty')?.textContent).toContain(ZH.empty);
    expect(host.querySelector('.assistant-send-button')?.textContent).toBe(ZH.send);

    // Switch back to Thai
    setLang('th');
    await flush();
    expect(host.querySelector('h1.assistant-title')?.textContent).toBe(TH.title);
  });

  it('6. amounts render through MoneyText and do not change with the locale', async () => {
    const viewData: MyAssistantConversationData = {
      conversation: { id: 'conv-money', locale: 'th', createdAt: '2026-10-13T03:00:00Z' },
      messages: [
        {
          id: 'msg-cust',
          role: 'CUSTOMER',
          redactedText: 'ต้องการแลก 100 USD ครับ',
          createdAt: '2026-10-13T03:00:01Z',
        },
        {
          id: 'msg-asst',
          role: 'ASSISTANT',
          redactedText: 'สำหรับ USD 100 ยอดที่ได้รับคือ 3512.50 THB ครับ',
          createdAt: '2026-10-13T03:00:02Z',
        },
      ],
    };

    const mockFetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ version: 1, changed: true, data: viewData }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );

    const host = mount({ fetch: mockFetch, initialConversationId: 'conv-money' });
    await flush();
    await flush();

    const bubbles = Array.from(host.querySelectorAll('.message-bubble'));
    expect(bubbles.length).toBe(2);

    // Customer bubble has 100 USD rendered through MoneyText
    expect(bubbles[0]?.textContent).toContain('100 USD');

    // Assistant bubble has 100 USD and 3,512.50 THB formatted with comma
    expect(bubbles[1]?.textContent).toContain('100 USD');
    expect(bubbles[1]?.textContent).toContain('3,512.50 THB');

    // Switch language to English and Chinese - amounts stay exactly the same
    setLang('en');
    await flush();
    expect(bubbles[1]?.textContent).toContain('3,512.50 THB');

    setLang('zh');
    await flush();
    expect(bubbles[1]?.textContent).toContain('3,512.50 THB');
  });

  it('7. sending a message: calls POST /api/app/assistant/messages with idempotency key, displays customer and assistant messages', async () => {
    let capturedBody: string | null = null;
    let capturedIdemKey: string | null = null;

    const mockFetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
      if (url === '/api/app/assistant/messages') {
        capturedBody = String(init?.body);
        const headers = new Headers(init?.headers);
        capturedIdemKey = headers.get('idempotency-key');

        const answer: AssistantAskAnswerDto = {
          conversationId: 'conv-new-1',
          customerMessage: {
            id: 'cm-1',
            role: 'CUSTOMER',
            redactedText: 'ขอทราบเรท USD',
            createdAt: '2026-10-13T03:10:00Z',
          },
          assistantReply: {
            id: 'am-1',
            role: 'ASSISTANT',
            redactedText: 'เรท USD รับซื้อ 35.12 THB ขาย 35.40 THB ครับ',
            createdAt: '2026-10-13T03:10:01Z',
          },
        };

        return Promise.resolve(
          new Response(JSON.stringify(answer), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }
      return Promise.reject(new Error(`Unhandled: ${url}`));
    });

    const host = mount({ fetch: mockFetch, branch: 'XC-B01' });
    await flush();

    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    const sendButton = host.querySelector('.assistant-send-button') as HTMLButtonElement;

    // Type input
    input.value = 'ขอทราบเรท USD';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    // Submit form
    sendButton.click();
    await flush();
    await flush();

    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(capturedIdemKey).toBeTruthy();
    expect(capturedBody).toBeTruthy();
    const parsedBody = JSON.parse(capturedBody!) as AssistantAskInput;
    expect(parsedBody.text).toBe('ขอทราบเรท USD');
    expect(parsedBody.branch).toBe('XC-B01');
    expect(parsedBody.locale).toBe('th');

    // Messages appear in conversation
    const messageBubbles = Array.from(host.querySelectorAll('.message-bubble'));
    expect(messageBubbles.length).toBe(2);
    expect(messageBubbles[0]?.textContent).toContain('ขอทราบเรท USD');
    expect(messageBubbles[1]?.textContent).toContain('35.12 THB');
    expect(messageBubbles[1]?.textContent).toContain('35.40 THB');

    // Input was cleared on success
    expect(input.value).toBe('');
  });

  it('8. clicking suggestion prompt sends that prompt as a message', async () => {
    let capturedBody: string | null = null;
    const mockFetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
      if (url === '/api/app/assistant/messages') {
        capturedBody = String(init?.body);
        const answer: AssistantAskAnswerDto = {
          conversationId: 'conv-prompt',
          customerMessage: {
            id: 'cm-2',
            role: 'CUSTOMER',
            redactedText: TH.promptRates,
            createdAt: '2026-10-13T03:15:00Z',
          },
          assistantReply: {
            id: 'am-2',
            role: 'ASSISTANT',
            redactedText: 'สามารถดูเรทสดได้ที่หน้าเรทครับ',
            createdAt: '2026-10-13T03:15:01Z',
          },
        };
        return Promise.resolve(
          new Response(JSON.stringify(answer), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }
      return Promise.reject(new Error(`Unhandled: ${url}`));
    });

    const host = mount({ fetch: mockFetch });
    await flush();

    const promptBtn = host.querySelector('.assistant-prompt-button') as HTMLButtonElement;
    expect(promptBtn.textContent).toBe(TH.promptRates);

    promptBtn.click();
    await flush();
    await flush();

    expect(capturedBody).toBeTruthy();
    const parsed = JSON.parse(capturedBody!) as AssistantAskInput;
    expect(parsed.text).toBe(TH.promptRates);

    const messageBubbles = Array.from(host.querySelectorAll('.message-bubble'));
    expect(messageBubbles.length).toBe(2);
  });

  it('9. send error: shows error alert when sending fails and retains input text', async () => {
    const mockFetch = vi.fn().mockImplementation(() =>
      Promise.resolve(
        new Response(
          JSON.stringify({ error: { code: 'rateLimited', message: 'ส่งข้อความเร็วเกินไป' } }),
          {
            status: 429,
            headers: { 'content-type': 'application/json' },
          },
        ),
      ),
    );

    const host = mount({ fetch: mockFetch });
    await flush();

    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    const sendButton = host.querySelector('.assistant-send-button') as HTMLButtonElement;

    input.value = 'ทดสอบข้อความ';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    sendButton.click();
    await flush();
    await flush();

    const sendError = host.querySelector('.assistant-send-error');
    expect(sendError).not.toBeNull();
    expect(sendError?.textContent).toContain('ส่งข้อความเร็วเกินไป');

    // Input text is retained upon failure!
    expect(input.value).toBe('ทดสอบข้อความ');
  });

  it('10. retrying the same failed message preserves the idempotency key', async () => {
    const capturedKeys: string[] = [];
    let attempts = 0;

    const mockFetch = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
      if (url === '/api/app/assistant/messages') {
        attempts++;
        const headers = new Headers(init?.headers);
        const key = headers.get('idempotency-key') ?? '';
        capturedKeys.push(key);

        if (attempts === 1) {
          return Promise.resolve(
            new Response(JSON.stringify({ error: { code: 'networkError', message: 'Timeout' } }), {
              status: 504,
              headers: { 'content-type': 'application/json' },
            }),
          );
        }

        const answer: AssistantAskAnswerDto = {
          conversationId: 'conv-retry-idem',
          customerMessage: {
            id: 'cm-r',
            role: 'CUSTOMER',
            redactedText: 'ลองใหม่อีกครั้ง',
            createdAt: '2026-10-13T03:20:00Z',
          },
          assistantReply: {
            id: 'am-r',
            role: 'ASSISTANT',
            redactedText: 'ได้รับแล้วครับ',
            createdAt: '2026-10-13T03:20:01Z',
          },
        };
        return Promise.resolve(
          new Response(JSON.stringify(answer), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }
      return Promise.reject(new Error(`Unhandled: ${url}`));
    });

    const host = mount({ fetch: mockFetch });
    await flush();

    const input = host.querySelector('.assistant-input') as HTMLInputElement;
    const sendButton = host.querySelector('.assistant-send-button') as HTMLButtonElement;

    input.value = 'ลองใหม่อีกครั้ง';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    // First attempt fails
    sendButton.click();
    await flush();
    await flush();

    expect(attempts).toBe(1);
    expect(input.value).toBe('ลองใหม่อีกครั้ง');

    // Second attempt with the same text
    sendButton.click();
    await flush();
    await flush();

    expect(attempts).toBe(2);
    expect(capturedKeys.length).toBe(2);
    expect(capturedKeys[0]).toBeTruthy();
    // Same idempotency key reused on retry of same text!
    expect(capturedKeys[1]).toBe(capturedKeys[0]);
    expect(input.value).toBe('');
  });

  it('11. HTML entry point web/pages/app-assistant.html has correct meta tags and gate', () => {
    const htmlPath = dir + '../../../../pages/app-assistant.html';
    expect(fs.existsSync(htmlPath)).toBe(true);

    const content = fs.readFileSync(htmlPath, 'utf-8');
    expect(content).toContain('<meta name="xc-path" content="/app/assistant">');
    expect(content).toContain('<meta name="xc-gate" content="anyone">');
    expect(content).toContain('<script type="module" src="/src/screens/customer/assistant/main.tsx"></script>');
  });

  it('12. main.tsx calls setAudience(\'customer\') before render', () => {
    const mainPath = dir + 'main.tsx';
    expect(fs.existsSync(mainPath)).toBe(true);

    const content = fs.readFileSync(mainPath, 'utf-8');
    const setAudienceIdx = content.indexOf("setAudience('customer')");
    const renderIdx = content.indexOf('render(');

    expect(setAudienceIdx).toBeGreaterThan(-1);
    expect(renderIdx).toBeGreaterThan(setAudienceIdx);
  });

  it('13. i18n tables th, en, and zh have identical keys and placeholders', () => {
    const thKeys = Object.keys(table.th).sort();
    const enKeys = Object.keys(table.en).sort();
    const zhKeys = Object.keys(table.zh).sort();

    expect(enKeys).toEqual(thKeys);
    expect(zhKeys).toEqual(thKeys);

    for (const key of thKeys) {
      const thVal = (table.th as Texts)[key]!;
      const enVal = (table.en as Texts)[key]!;
      const zhVal = (table.zh as Texts)[key]!;

      expect(thVal.trim()).not.toBe('');
      expect(enVal.trim()).not.toBe('');
      expect(zhVal.trim()).not.toBe('');

      const thPlaceholders = [...thVal.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      const enPlaceholders = [...enVal.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      const zhPlaceholders = [...zhVal.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

      expect(enPlaceholders).toEqual(thPlaceholders);
      expect(zhPlaceholders).toEqual(thPlaceholders);
    }
  });
});
