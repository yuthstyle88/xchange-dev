// #2565: Customer assistant amounts with thousands separators must render as a single MoneyText
// span without splitting at the comma or leaving trailing decimals behind.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeAssistantConversationDto,
  makeAssistantMessageDto,
  makeMyAssistantConversationData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { AssistantScreen, renderTextWithMoney } from './AssistantScreen';

let dispose: (() => void) | undefined;

function mountReply(redactedText: string): Promise<HTMLElement> {
  const viewData = makeMyAssistantConversationData({
    conversation: makeAssistantConversationDto({ id: 'conv-2565', locale: 'th', createdAt: '2026-10-16T03:00:00Z' }),
    messages: [
      makeAssistantMessageDto({
        id: 'msg-1',
        role: 'ASSISTANT',
        redactedText,
        createdAt: '2026-10-16T03:00:01Z',
      }),
    ],
  });

  const mockFetch = vi.fn().mockResolvedValue(
    new Response(JSON.stringify({ version: 1, changed: true, data: viewData }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }),
  );

  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <AssistantScreen fetch={mockFetch} initialConversationId="conv-2565" />,
    host,
  );

  return new Promise<HTMLElement>((resolve) => {
    setTimeout(() => {
      setTimeout(() => {
        const bubble = host.querySelector<HTMLElement>('.message-bubble');
        expect(bubble, 'assistant message bubble').not.toBeNull();
        resolve(bubble!);
      }, 0);
    }, 0);
  });
}

function renderFragments(text: string): string[] {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <>{renderTextWithMoney(text)}</>, host);
  return [...host.querySelectorAll('span')].map((s) => s.textContent ?? '');
}

beforeEach(() => {
  setAudience('customer');
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  setLang('th');
  setAudience('staff');
  vi.restoreAllMocks();
});

describe('#2565 AssistantScreen money formatting with thousands separators', () => {
  it('row 1: Reply text "สำหรับ USD 1,234.56 ครับ" -> Bubble shows "1,234.56 USD" once and no stray ".56"', async () => {
    const bubble = await mountReply('สำหรับ USD 1,234.56 ครับ');

    expect(bubble.textContent).toBe('สำหรับ 1,234.56 USD ครับ');
    expect(bubble.textContent).not.toContain('USD.56');
    expect([...bubble.querySelectorAll('span')].map((s) => s.textContent)).toEqual([
      'สำหรับ ',
      '1,234.56 USD',
      ' ครับ',
    ]);
  });

  it('row 2: Reply text "THB 12,500.00" -> Shows "12,500.00 THB"', async () => {
    const bubble = await mountReply('THB 12,500.00');

    expect(bubble.textContent).toBe('12,500.00 THB');
    expect(bubble.textContent).not.toContain('THB.00');
    expect([...bubble.querySelectorAll('span')].map((s) => s.textContent)).toEqual([
      '12,500.00 THB',
    ]);
  });

  it('row 3: Reply text "1,234.56 USD" -> Shows "1,234.56 USD" as a single MoneyText span without splitting "1,"', async () => {
    const bubble = await mountReply('1,234.56 USD');

    expect(bubble.textContent).toBe('1,234.56 USD');
    expect([...bubble.querySelectorAll('span')].map((s) => s.textContent)).toEqual([
      '1,234.56 USD',
    ]);
  });

  it('edge: renderTextWithMoney keeps number-between-two-currencies behaviour ("USD 35.12 THB" and "USD 1,234.56 THB")', () => {
    expect(renderFragments('USD 35.12 THB')).toEqual(['35.12 USD', ' THB']);
    expect(renderFragments('USD 1,234.56 THB')).toEqual(['1,234.56 USD', ' THB']);
  });
});
