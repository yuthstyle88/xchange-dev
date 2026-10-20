import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { AssistantScreen } from './AssistantScreen';
import {
  makeAssistantConversationDto,
  makeAssistantMessageDto,
  makeMyAssistantConversationData,
} from '../../../shared/testing/fixtures';

describe('WE-28: screens/customer/assistant/ — wordedError handling (#1777)', () => {
  let host: HTMLDivElement;
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
    setLang('th');
    setAudience('staff');
    vi.restoreAllMocks();
  });

  function flush() {
    return new Promise((resolve) => setTimeout(resolve, 0));
  }

  function mount(props: Parameters<typeof AssistantScreen>[0] = {}) {
    dispose = render(() => <AssistantScreen {...props} />, host);
    return host;
  }

  function getLoadError(): string | null {
    const el = host.querySelector('[role="alert"].assistant-error p');
    return el ? (el.textContent ?? '').trim() : null;
  }

  function getSendError(): string | null {
    const el = host.querySelector('[role="alert"].assistant-send-error');
    return el ? (el.textContent ?? '').trim() : null;
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: loadConversation (my_assistant_conversation view)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: loadConversation', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      const mockFetch = vi.fn().mockImplementation(() =>
        Promise.reject(new TypeError('Failed to fetch')),
      );

      mount({ fetch: mockFetch, initialConversationId: 'c-net-1' });
      await vi.waitFor(() => {
        expect(getLoadError()).toBe(th['error.network']);
      });
      expect(getLoadError()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      mount({ fetch: mockFetch, initialConversationId: 'c-net-1' });
      await vi.waitFor(() => {
        expect(getLoadError()).toBe(en['error.network']);
      });
      expect(getLoadError()).not.toContain('Failed to fetch');
    });

    it('(2) 500 refusal shows error.serverError in TH (keyed non-server code)', async () => {
      const mockFetch = vi.fn().mockImplementation(() =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              error: { code: 'roleNotAllowedHere', message: 'Internal DB fault' },
            }),
            { status: 500, headers: { 'content-type': 'application/json' } },
          ),
        ),
      );

      mount({ fetch: mockFetch, initialConversationId: 'c-500-1' });
      await vi.waitFor(() => {
        expect(getLoadError()).toBe(th['error.serverError']);
      });
      expect(getLoadError()).not.toContain('Internal DB fault');
      expect(getLoadError()).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const serverMsg = 'ข้อความขัดข้องเฉพาะจากเซิร์ฟเวอร์สำหรับบทสนทนา';
      const mockFetch = vi.fn().mockImplementation(() =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              error: { code: 'unkeyedAssistantCode', message: serverMsg },
            }),
            { status: 400, headers: { 'content-type': 'application/json' } },
          ),
        ),
      );

      mount({ fetch: mockFetch, initialConversationId: 'c-unk-1' });
      await vi.waitFor(() => {
        expect(getLoadError()).toBe(serverMsg);
      });
      expect(getLoadError()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 2: sendMessage (/api/app/assistant/messages command)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 2: sendMessage', () => {
    async function triggerSend(mockFetch: (input: string, init?: RequestInit) => Promise<Response>) {
      mount({ fetch: mockFetch });
      await flush();

      const input = host.querySelector('.assistant-input') as HTMLInputElement;
      input.value = 'สอบถามเรท';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      await flush();

      const sendBtn = host.querySelector('.assistant-send-button') as HTMLButtonElement;
      sendBtn.click();
      await flush();
      await flush();
    }

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      const mockFetch = vi.fn().mockImplementation(() =>
        Promise.reject(new TypeError('Failed to fetch')),
      );

      await triggerSend(mockFetch);
      await vi.waitFor(() => {
        expect(getSendError()).toBe(th['error.network']);
      });
      expect(getSendError()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      await triggerSend(mockFetch);
      await vi.waitFor(() => {
        expect(getSendError()).toBe(en['error.network']);
      });
      expect(getSendError()).not.toContain('Failed to fetch');
    });

    it('(2) 500 refusal shows error.serverError in TH (keyed non-server code)', async () => {
      const mockFetch = vi.fn().mockImplementation(() =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              error: { code: 'roleNotAllowedHere', message: 'Internal DB fault on send' },
            }),
            { status: 500, headers: { 'content-type': 'application/json' } },
          ),
        ),
      );

      await triggerSend(mockFetch);
      await vi.waitFor(() => {
        expect(getSendError()).toBe(th['error.serverError']);
      });
      expect(getSendError()).not.toContain('Internal DB fault on send');
      expect(getSendError()).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const serverMsg = 'ข้อความส่งคำถามถูกปฏิเสธจากเซิร์ฟเวอร์';
      const mockFetch = vi.fn().mockImplementation(() =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              error: { code: 'unkeyedSendRefusalCode', message: serverMsg },
            }),
            { status: 400, headers: { 'content-type': 'application/json' } },
          ),
        ),
      );

      await triggerSend(mockFetch);
      await vi.waitFor(() => {
        expect(getSendError()).toBe(serverMsg);
      });
      expect(getSendError()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Criterion 3: Stale messages cleared when conversation load fails / resets
  // ──────────────────────────────────────────────────────────────────────────
  describe('Criterion 3: stale data isolation across reset and error', () => {
    it('shows error without stale message container on failed initial load, recovers on retry, and clears on reset', async () => {
      let callCount = 0;
      const initialData = makeMyAssistantConversationData({
        conversation: makeAssistantConversationDto({ id: 'c-prev', locale: 'th', createdAt: '2026-10-15T00:00:00Z' }),
        messages: [
          makeAssistantMessageDto({
            id: 'm-1',
            role: 'ASSISTANT',
            redactedText: 'ยินดีต้อนรับสู่บริการสนทนา',
            createdAt: '2026-10-15T00:00:01Z',
          }),
        ],
      });

      const mockFetch = vi.fn().mockImplementation(() => {
        callCount++;
        if (callCount === 1) {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                error: { code: 'roleNotAllowedHere', message: 'First attempt failed' },
              }),
              { status: 500, headers: { 'content-type': 'application/json' } },
            ),
          );
        }
        return Promise.resolve(
          new Response(
            JSON.stringify({ version: 1, changed: true, data: initialData }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          ),
        );
      });

      mount({ fetch: mockFetch, initialConversationId: 'c-prev' });
      await vi.waitFor(() => {
        expect(getLoadError()).toBe(th['error.serverError']);
      });
      // No messages rendered on failed load
      expect(host.querySelector('.assistant-messages')).toBeNull();

      // Click retry
      const retryBtn = host.querySelector('.assistant-retry-button') as HTMLButtonElement;
      expect(retryBtn).not.toBeNull();
      retryBtn.click();

      // Succeeded: messages are shown
      await vi.waitFor(() => {
        expect(host.querySelector('.assistant-messages')).not.toBeNull();
        expect(host.textContent).toContain('ยินดีต้อนรับสู่บริการสนทนา');
      });

      // Reset conversation clears stale messages
      const resetBtn = host.querySelector('.assistant-reset-button') as HTMLButtonElement;
      expect(resetBtn).not.toBeNull();
      resetBtn.click();
      await flush();

      expect(host.querySelector('.assistant-messages')).toBeNull();
      expect(host.textContent).not.toContain('ยินดีต้อนรับสู่บริการสนทนา');
    });
  });
});
