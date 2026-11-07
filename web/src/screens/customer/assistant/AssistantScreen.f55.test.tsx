// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// Do not edit, delete or weaken these tests.
//
// A chat message's time is an instant a person reads, and the message list has no day heading (a resumed
// conversation can be from an earlier day), so it is shown with formatBangkokDateTime, whatever the page language.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeAssistantConversationDto,
  makeAssistantMessageDto,
  makeMyAssistantConversationData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { AssistantScreen } from './AssistantScreen';

let dispose: (() => void) | undefined;

function mountMessages(): Promise<string[]> {
  const viewData = makeMyAssistantConversationData({
    conversation: makeAssistantConversationDto({ id: 'conv-f55', locale: 'th', createdAt: '2026-11-04T06:23:00Z' }),
    messages: [
      makeAssistantMessageDto({ id: 'm1', role: 'CUSTOMER', redactedText: 'hello', createdAt: '2026-11-04T06:23:45Z' }),
      makeAssistantMessageDto({ id: 'm2', role: 'ASSISTANT', redactedText: 'ok', createdAt: '2026-11-04T17:05:00Z' }),
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
  dispose = render(() => <AssistantScreen fetch={mockFetch} initialConversationId="conv-f55" />, host);
  return new Promise<string[]>((resolve) => {
    setTimeout(() => {
      setTimeout(() => {
        resolve([...host.querySelectorAll('.message-time')].map((el) => el.textContent ?? ''));
      }, 0);
    }, 0);
  });
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

describe('F55 AssistantScreen message time', () => {
  it('shows each message time as dd/mm/yyyy HH:mm in Bangkok (CE year, seconds dropped)', async () => {
    // 06:23:45Z = 13:23 ICT on 04/11; 17:05Z = 00:05 ICT on the next day.
    expect(await mountMessages()).toEqual(['04/11/2026 13:23', '05/11/2026 00:05']);
  });

  it('prints the same text in English (no page-locale formatting)', async () => {
    setLang('en');
    expect(await mountMessages()).toEqual(['04/11/2026 13:23', '05/11/2026 00:05']);
  });
});
