// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// Do not edit, delete or weaken these tests.
//
// A plan draft's expiry is a day, so it is shown with formatBangkokDate (dd/mm/yyyy, CE, Bangkok),
// not toLocaleDateString(locale(), { month: 'short' }) (`5 พ.ย. 2642`).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeMyPlannerDraftsData, makePlannerDraftDto } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { JourneyScreen } from './JourneyScreen';

const DRAFT = makePlannerDraftDto({
  id: 'draft-f55',
  branch: 'SIAM',
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '100.00' },
  version: 1,
  // 17:30Z on 04/11 is 00:30 ICT on 05/11: the Bangkok day, not the UTC one.
  expiresAt: '2099-11-04T17:30:00Z',
  createdAt: '2099-11-01T10:00:00Z',
  updatedAt: '2099-11-01T10:00:00Z',
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

async function expiresText(): Promise<string> {
  const testFetch = async (url: string): Promise<Response> => {
    if (url.startsWith('/api/v3/views/my_planner_drafts')) {
      return json(200, { version: 1, changed: true, data: makeMyPlannerDraftsData({ drafts: [DRAFT] }) });
    }
    return json(404, { error: { code: 'notFound', message: url } });
  };
  dispose = render(() => <JourneyScreen fetch={testFetch} />, host);
  await flush();
  return host.querySelector('.draft-expires')?.textContent ?? '';
}

describe('F55 JourneyScreen draft expiry', () => {
  beforeAll(() => warmModules(() => import('./JourneyScreen')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.restoreAllMocks();
    setLang('th');
  });

  it('Thai: the expiry reads dd/mm/yyyy, CE year, Bangkok day', async () => {
    expect(await expiresText()).toBe('หมดอายุ: 05/11/2099');
  });

  it('English: the same date text', async () => {
    setLang('en');
    expect(await expiresText()).toBe('Expires: 05/11/2099');
  });
});
