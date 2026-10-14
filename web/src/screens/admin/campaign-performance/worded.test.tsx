import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { CampaignPerformance } from './CampaignPerformance';
import type { AdminMenuProps } from '../menus';

function dummyProps(overrides?: Partial<AdminMenuProps>): AdminMenuProps {
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [branch] = createSignal('BKK');
  const [shown] = createSignal(true);
  return {
    period,
    branch,
    shown,
    ...overrides,
  };
}

describe('WE-7: screens/admin/campaign-performance/ worded error tests (web-worded-error.md)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
  });

  afterEach(async () => {
    if (dispose) {
      dispose();
      dispose = null;
    }
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }

  // (1) a network TypeError shows th['error.network'], and after setLang('en') shows en['error.network']
  it('(1) network TypeError shows th error.network and en error.network after setLang', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));

    // Thai (default)
    setLang('th');
    dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
    await settle();

    expect(container.textContent).toContain(th['error.network']);
    expect(container.textContent).not.toContain('Failed to fetch');
    expect(container.textContent).not.toContain('TypeError');

    dispose();
    dispose = null;
    container.innerHTML = '';

    // English
    setLang('en');
    dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
    await settle();

    expect(container.textContent).toContain(en['error.network']);
    expect(container.textContent).not.toContain('Failed to fetch');
    expect(container.textContent).not.toContain('TypeError');
  });

  // (2) a 500 refusal shows th['error.serverError']
  it('(2) a 500 refusal shows th error.serverError', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => {
      return new Response(
        JSON.stringify({
          error: {
            code: 'serverError',
            message: 'Internal Database Crash on Node 42',
          },
        }),
        {
          status: 500,
          headers: { 'content-type': 'application/json' },
        },
      );
    }));

    setLang('th');
    dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
    await settle();

    expect(container.textContent).toContain(th['error.serverError']);
    expect(container.textContent).not.toContain('Internal Database Crash on Node 42');
  });

  // (3) a refusal with a code that has no shared key and a server message shows that message (pins W10 (4))
  it('(3) a refusal with an unkeyed code and a server message shows that message', async () => {
    const serverSentence = 'แคมเปญนี้ยังไม่ได้รับการอนุมัติจากผู้จัดการฝ่ายการตลาด';
    vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => {
      return new Response(
        JSON.stringify({
          error: {
            code: 'campaignNotYetApprovedByManager',
            message: serverSentence,
          },
        }),
        {
          status: 400,
          headers: { 'content-type': 'application/json' },
        },
      );
    }));

    setLang('th');
    dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
    await settle();

    // Must show the server's own sentence, and must not be replaced by generic server error
    expect(container.textContent).toContain(serverSentence);
    expect(container.textContent).not.toContain(th['error.serverError']);
  });
});
