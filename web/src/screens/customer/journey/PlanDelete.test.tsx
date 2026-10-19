// C09 up-front tests (feat/fe-customer skeleton): PlanDelete deletes a saved plan with plannerDraftDelete
// `POST /api/app/planner-drafts/{id}/delete`. The implementer makes these pass and must not edit them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import type { PlannerDraftDto } from '../../../shared/api/generated/my_planner_drafts';
import { setLang } from '../../../shared/i18n';
import { PlanDelete } from './PlanDelete';

type Reply = { status: number; body: unknown } | 'network';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const DRAFT = {
  id: 'pd-1',
  branch: null,
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '100' },
  expiresAt: '2099-01-01T00:00:00Z',
} as unknown as PlannerDraftDto;

function server(replies: Reply[] = []) {
  const writes: { path: string; key: string | null; body: unknown }[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const path = new URL(input, 'http://localhost').pathname;
    if ((init?.method ?? 'GET').toUpperCase() !== 'POST' || path !== '/api/app/planner-drafts/pd-1/delete') {
      return json(404, { error: { code: 'notFound', message: 'Not found' } });
    }
    writes.push({ path, key: new Headers(init?.headers).get('idempotency-key'), body: JSON.parse(String(init?.body ?? '{}')) });
    const r = replies.shift() ?? { status: 200, body: { id: 'pd-1' } };
    if (r === 'network') throw new TypeError('Failed to fetch');
    return json(r.status, r.body);
  });
  return { api: new ApiClient(fetchFn), writes };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
function mount(api: ApiClient, onDeleted = vi.fn()) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <PlanDelete draft={DRAFT} api={api} onDeleted={onDeleted} />, host);
  return onDeleted;
}
const button = () => host.querySelector('button[data-action="delete-plan"]') as HTMLButtonElement | null;

beforeEach(() => setLang('en'));
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('PlanDelete (C09)', () => {
  it('posts {id} to /api/app/planner-drafts/{id}/delete with an Idempotency-Key, then reports the id', async () => {
    const s = server();
    const onDeleted = mount(s.api);
    button()!.click();
    await flush();
    expect(s.writes).toEqual([{ path: '/api/app/planner-drafts/pd-1/delete', key: expect.any(String), body: { id: 'pd-1' } }]);
    expect(s.writes[0].key).toBeTruthy();
    expect(onDeleted).toHaveBeenCalledWith('pd-1');
  });

  it('the button is disabled while the delete is in flight, so one click sends one write', async () => {
    const s = server();
    mount(s.api);
    button()!.click();
    expect(button()!.disabled).toBe(true);
    button()!.click();
    await flush();
    expect(s.writes).toHaveLength(1);
  });

  it('a failed delete shows the error, keeps the plan, and the retry reuses the key', async () => {
    const s = server(['network']);
    const onDeleted = mount(s.api);
    button()!.click();
    await flush();
    expect(host.querySelector('[data-testid="plan-delete-error"]')?.textContent?.trim()).toBeTruthy();
    expect(onDeleted).not.toHaveBeenCalled();
    button()!.click();
    await flush();
    expect(s.writes).toHaveLength(2);
    expect(s.writes[1].key).toBe(s.writes[0].key);
    expect(onDeleted).toHaveBeenCalledWith('pd-1');
  });

  it('a 404 (already deleted) still reports the id, so the plan leaves the list', async () => {
    const onDeleted = mount(server([{ status: 404, body: { error: { code: 'notFound', message: 'gone' } } }]).api);
    button()!.click();
    await flush();
    expect(onDeleted).toHaveBeenCalledWith('pd-1');
  });
});
