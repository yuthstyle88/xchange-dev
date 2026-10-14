import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { th } from '../../shared/i18n/th';
import { en } from '../../shared/i18n/en';
import { ApiClient } from '../../shared/api/client';
import type { StaffMeView } from '../../shared/api/generated/staff_me';
import { StaffPage } from './StaffPage';

describe('StaffPage: wordedError on failed calls (WE-38 #1787)', () => {
  let host: HTMLDivElement | undefined;
  let dispose: (() => void) | undefined;

  const staffMe: StaffMeView = {
    userId: '00000000-0000-4000-8000-0000000000a1',
    name: 'Ploy',
    roles: [],
    branchId: 'MAIN',
    branchName: 'XC-B',
    permissions: [],
  };

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host?.remove();
    host = undefined;
    setLang('th');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function mountWithFloorReply(floorFetch: () => Promise<Response>) {
    const fetchFn = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes('/api/v3/views/staff_me')) {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: staffMe }),
          { status: 200, headers: { 'content-type': 'application/json' } }
        );
      }
      if (url.includes('/api/v3/views/staff_floor')) {
        return floorFetch();
      }
      throw new Error(`Unexpected url: ${url}`);
    });

    const api = new ApiClient(fetchFn as unknown as typeof fetch);
    dispose = render(() => <StaffPage api={api} cards={[]} navigate={() => {}} />, host!);
  }

  it('(1) a network TypeError shows error.network in TH and EN', async () => {
    mountWithFloorReply(() => Promise.reject(new TypeError('Failed to fetch')));

    await vi.waitFor(() => {
      const alert = host!.querySelector('[role="alert"]');
      expect(alert, 'error alert displayed in TH').not.toBeNull();
      expect(alert!.textContent).toBe(th['error.network']);
    });

    // In English
    setLang('en');
    host!.innerHTML = '';
    dispose?.();
    mountWithFloorReply(() => Promise.reject(new TypeError('Failed to fetch')));

    await vi.waitFor(() => {
      const alert = host!.querySelector('[role="alert"]');
      expect(alert, 'error alert displayed in EN').not.toBeNull();
      expect(alert!.textContent).toBe(en['error.network']);
    });
  });

  it('(2) a 500 refusal shows error.serverError in TH', async () => {
    mountWithFloorReply(async () =>
      new Response(
        JSON.stringify({ error: { code: 'roleNotAllowedHere', message: 'internal db timeout' } }),
        { status: 500, headers: { 'content-type': 'application/json' } }
      )
    );

    await vi.waitFor(() => {
      const alert = host!.querySelector('[role="alert"]');
      expect(alert, 'error alert displayed').not.toBeNull();
      expect(alert!.textContent).toBe(th['error.serverError']);
      expect(alert!.textContent).not.toContain('internal db timeout');
    });
  });

  it('(3) an unknown code with a server message shows that message', async () => {
    const serverSentence = 'สาขาปิดทำการชั่วคราวเนื่องจากระบบขัดข้อง';
    mountWithFloorReply(async () =>
      new Response(
        JSON.stringify({ error: { code: 'branchClosedTemporarily', message: serverSentence } }),
        { status: 400, headers: { 'content-type': 'application/json' } }
      )
    );

    await vi.waitFor(() => {
      const alert = host!.querySelector('[role="alert"]');
      expect(alert, 'error alert displayed').not.toBeNull();
      expect(alert!.textContent).toBe(serverSentence);
    });
  });
});
