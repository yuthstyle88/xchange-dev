// C12 up-front tests (feat/fe-customer-2 skeleton): AnswerPreview shows what the assistant understood (BE contract
// #3407: `intent` + `preview` on `AssistantAskAnswerDto`): a QUOTE preview the customer confirms as a planner draft
// (`POST /api/app/planner-drafts`), or the link the intent's kind leads to (derived on the client, PR 3387 S4).
// The implementer makes these pass and must not edit them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import {
  makeAssistantAskAnswerDto,
  makeAssistantIntentDto,
  type AssistantAskAnswerDto,
  type AssistantIntentDto,
} from '../../../shared/testing/fixtures/assistant_ask';
import { makeAssistantMessageDto } from '../../../shared/testing/fixtures/my_assistant_conversation';
import { setLang } from '../../../shared/i18n';
import { AnswerPreview } from './AnswerPreview';

type Reply = { status: number; body: unknown } | 'network';

const BASE = makeAssistantAskAnswerDto({
  conversationId: 'c-1',
  customerMessage: makeAssistantMessageDto({ id: 'm1', role: 'CUSTOMER', redactedText: 'แลก 100 USD', createdAt: '2026-10-19T08:00:00Z' }),
  assistantReply: makeAssistantMessageDto({ id: 'm2', role: 'ASSISTANT', redactedText: 'ok', createdAt: '2026-10-19T08:00:01Z' }),
  intent: makeAssistantIntentDto({ kind: 'UNKNOWN' }),
  preview: null,
});
const PREVIEW = { thb: '3650.00', rate: '36.50', unitSize: '1', stale: false, sourceDate: null };
const quote = (over: Partial<AssistantIntentDto> = {}, preview: AssistantAskAnswerDto['preview'] = PREVIEW) => ({
  ...BASE,
  intent: { kind: 'QUOTE' as const, currency: 'USD', customerSide: 'CUSTOMER_SELL', quantity: '100', ...over },
  preview,
});

function server(replies: Reply[] = []) {
  const writes: { path: string; key: string | null; body: Record<string, unknown> }[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const path = new URL(input, 'http://localhost').pathname;
    if ((init?.method ?? 'GET').toUpperCase() !== 'POST' || path !== '/api/app/planner-drafts') {
      return new Response('{}', { status: 404 });
    }
    writes.push({ path, key: new Headers(init?.headers).get('idempotency-key'), body: JSON.parse(String(init?.body ?? '{}')) });
    const r = replies.shift() ?? { status: 200, body: { id: 'pd-1', version: 1 } };
    if (r === 'network') throw new TypeError('Failed to fetch');
    return new Response(JSON.stringify(r.body), { status: r.status, headers: { 'content-type': 'application/json' } });
  });
  return { api: new ApiClient(fetchFn), writes };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
function mount(answer: AssistantAskAnswerDto, api: ApiClient, branch?: string): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <AnswerPreview answer={answer} api={api} branch={branch} />, host);
}
const confirm = () => host.querySelector('button[data-action="confirm-plan"]') as HTMLButtonElement | null;
const links = () => Array.from(host.querySelectorAll('a')).map((a) => a.getAttribute('href'));

beforeEach(() => setLang('en'));
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('AnswerPreview (C12)', () => {
  it('a QUOTE with a preview shows the THB amount and a confirm button', () => {
    mount(quote(), server().api, 'B01');
    expect(host.querySelector('[data-testid="preview-thb"]')?.textContent).toMatch(/3,?650/);
    expect(confirm()).not.toBeNull();
    expect(host.querySelector('[data-testid="preview-stale"]')).toBeNull();
  });

  it('a stale preview says so', () => {
    mount(quote({}, { ...PREVIEW, stale: true }), server().api, 'B01');
    expect(host.querySelector('[data-testid="preview-stale"]')).not.toBeNull();
  });

  it('confirm saves a planner draft from the intent, keyed, then says saved and links to the plans', async () => {
    const s = server();
    mount(quote(), s.api, 'B01');
    confirm()!.click();
    await flush();
    expect(s.writes).toHaveLength(1);
    expect(s.writes[0].body).toEqual({ branch: 'B01', currency: 'USD', customerSide: 'CUSTOMER_SELL', amount: { quantity: '100' } });
    expect(s.writes[0].key).toBeTruthy();
    expect(host.querySelector('[data-testid="preview-saved"]')).not.toBeNull();
    expect(links()).toContain('/app/journey');
    expect(confirm()).toBeNull();
  });

  it('a budget intent saves amount.budgetThb; the intent\'s own branch beats the screen\'s', async () => {
    const s = server();
    mount(quote({ quantity: undefined, budgetThb: '5000', branch: 'B02' }), s.api, 'B01');
    confirm()!.click();
    await flush();
    expect(s.writes[0].body).toEqual({ branch: 'B02', currency: 'USD', customerSide: 'CUSTOMER_SELL', amount: { budgetThb: '5000' } });
  });

  it('a failed save shows the error, keeps the button, and the retry reuses the key', async () => {
    const s = server(['network']);
    mount(quote(), s.api, 'B01');
    confirm()!.click();
    await flush();
    expect(host.querySelector('[data-testid="preview-error"]')?.textContent?.trim()).toBeTruthy();
    confirm()!.click();
    await flush();
    expect(s.writes).toHaveLength(2);
    expect(s.writes[1].key).toBe(s.writes[0].key);
  });

  it('a QUOTE with no preview (no published line) offers no confirm', () => {
    mount(quote({}, null), server().api, 'B01');
    expect(confirm()).toBeNull();
  });

  it('a QUOTE missing its currency or side offers no confirm (a draft cannot be built)', () => {
    mount(quote({ currency: undefined }), server().api, 'B01');
    expect(confirm()).toBeNull();
  });

  it.each([
    ['QUEUE', '/app/journey'],
    ['ALERT', '/app/rate-alerts'],
    ['BRANCH', '/app/branches'],
  ] as const)('a %s intent links to %s, with no preview', (kind, href) => {
    mount({ ...BASE, intent: { kind }, preview: null }, server().api);
    expect(links()).toEqual([href]);
    expect(confirm()).toBeNull();
  });

  it('an UNKNOWN intent, or an answer with no intent (an older back end), renders nothing', () => {
    mount({ ...BASE, intent: { kind: 'UNKNOWN' }, preview: null }, server().api);
    expect(host.textContent?.trim()).toBe('');
    dispose?.();
    host.remove();
    const { intent: _intent, preview: _preview, ...olderBackEnd } = BASE;
    mount(olderBackEnd as unknown as Parameters<typeof mount>[0], server().api);
    expect(host.textContent?.trim()).toBe('');
  });
});
