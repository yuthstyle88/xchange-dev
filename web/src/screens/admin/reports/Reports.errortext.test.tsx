import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en } from '../../../shared/i18n/en';
import { th } from '../../../shared/i18n/th';
import { Reports } from './Reports';

describe('Reports refusal errorText (#525)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let refusal: { status: number; body: unknown } | null = null;

  beforeEach(() => {
    setLang('th');
    refusal = null;
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const url = String(input);
        if (url.includes('/api/v3/views/admin_reports') && refusal) {
          return Promise.resolve(
            new Response(JSON.stringify(refusal.body), {
              status: refusal.status,
              headers: { 'content-type': 'application/json' },
            }),
          );
        }
        return Promise.resolve(
          new Response(JSON.stringify({ version: 1, changed: false }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): HTMLElement {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-05', to: '2026-10-06' });
    const [shown] = createSignal(true);
    dispose = render(
      () => <Reports branch={branch} period={period} shown={shown} />,
      root,
    );
    return root;
  }

  it('case 1: 500 serverError shows th error.serverError and not the raw message', async () => {
    refusal = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-525-not-a-table-sentence',
        },
      },
    };
    mount();
    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });
    const alertText = root.querySelector('[role="alert"]')!.textContent!;
    expect(alertText).toContain(th['error.serverError']);
    expect(alertText).not.toContain('RAW-525-not-a-table-sentence');
    expect(root.textContent).toContain(th['error.serverError']);
    expect(root.textContent).not.toContain('RAW-525-not-a-table-sentence');
  });

  it('case 2: the same answer after setLang("en") shows en error.serverError and not the raw message', async () => {
    setLang('en');
    refusal = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-525-not-a-table-sentence',
        },
      },
    };
    mount();
    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });
    const alertText = root.querySelector('[role="alert"]')!.textContent!;
    expect(alertText).toContain(en['error.serverError']);
    expect(alertText).not.toContain('RAW-525-not-a-table-sentence');
    expect(root.textContent).toContain(en['error.serverError']);
    expect(root.textContent).not.toContain('RAW-525-not-a-table-sentence');
  });

  it('case 3: 400 unknownCode525 passes through the raw server message', async () => {
    refusal = {
      status: 400,
      body: {
        error: {
          code: 'unknownCode525',
          message: 'RAW-525-passthrough-message',
        },
      },
    };
    mount();
    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });
    const alertText = root.querySelector('[role="alert"]')!.textContent!;
    expect(alertText).toContain('RAW-525-passthrough-message');
    expect(root.textContent).toContain('RAW-525-passthrough-message');
  });
});
