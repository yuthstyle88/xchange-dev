import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Toast, toast } from '../parts/toast';
import { DenominationGroups } from './DenominationGroups';
import * as table from './i18n';

interface SentWrite {
  path: string;
  method: string;
  key: string | null;
  body: Record<string, unknown>;
}

describe('BC-11: DenominationGroups note-add form', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let reads: number;
  let refusal: { status: number; code: string; message: string } | null;

  beforeEach(() => {
    setLang('th');
    toast('');
    writes = [];
    reads = 0;
    refusal = null;
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://desk.test');
        const path = url.pathname;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (path === '/api/v3/views/admin_denomination_groups' && method === 'GET') {
          reads += 1;
          return new Response(
            JSON.stringify({
              version: reads,
              changed: true,
              data: {
                provider: 'RATE_BOARD',
                currencies: [
                  {
                    currency: 'KRW',
                    groups: [
                      { group: 'ALL', instrumentKey: 'KRW', removable: false },
                      { group: 'KRWS', instrumentKey: null, removable: true },
                    ],
                  },
                ],
              },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }

        if (path === '/api/denomination-notes' && method === 'POST') {
          const headers = new Headers(init?.headers);
          const body = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : {};
          writes.push({
            path,
            method,
            key: headers.get('idempotency-key'),
            body,
          });
          if (refusal) {
            return new Response(
              JSON.stringify({ error: { code: refusal.code, message: refusal.message } }),
              { status: refusal.status, headers: { 'content-type': 'application/json' } },
            );
          }
          return new Response(
            JSON.stringify({
              id: 1,
              currency: body.currency,
              group: body.group,
              faceValue: body.faceValue,
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }

        return new Response(JSON.stringify({ version: 1, changed: false }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );
  });

  afterEach(async () => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    toast('');
    vi.unstubAllGlobals();
    setLang('th');
  });

  async function settle(): Promise<void> {
    for (let round = 0; round < 4; round++) {
      for (let i = 0; i < 5; i++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountAndPickKrw(): Promise<void> {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-11-04', to: '2026-11-04' });
    const [shown] = createSignal(true);
    dispose = render(
      () =>
        (
          <>
            <DenominationGroups branch={branch} period={period} shown={shown} />
            <Toast />
          </>
        ) as JSX.Element,
      root,
    );
    await settle();
    const select = root.querySelector<HTMLSelectElement>(
      '.denomination-groups-currency-picker select',
    )!;
    select.value = 'KRW';
    select.dispatchEvent(new Event('input', { bubbles: true }));
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
  }

  function setInput(el: HTMLInputElement | HTMLSelectElement, val: string): void {
    el.value = val;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  it('adds a note to the selected group via POST /api/denomination-notes and re-reads', async () => {
    await mountAndPickKrw();
    const beforeReads = reads;

    const groupSelect = root.querySelector<HTMLSelectElement>('.add-note-form select')!;
    const faceInput = root.querySelector<HTMLInputElement>('.add-note-form input')!;
    const addBtn = root.querySelector<HTMLButtonElement>('.add-note-form button')!;

    setInput(groupSelect, 'KRWS');
    setInput(faceInput, '1000');
    addBtn.click();
    await settle();

    expect(writes).toHaveLength(1);
    expect(writes[0]!.path).toBe('/api/denomination-notes');
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[0]!.body).toEqual({
      currency: 'KRW',
      group: 'KRWS',
      faceValue: '1000',
      branch: 'MAIN',
    });
    expect(reads).toBeGreaterThan(beforeReads);
    expect(faceInput.value).toBe('');
  });

  it('refuses non-positive or >4 decimal face values without sending', async () => {
    await mountAndPickKrw();
    const faceInput = root.querySelector<HTMLInputElement>('.add-note-form input')!;
    const addBtn = root.querySelector<HTMLButtonElement>('.add-note-form button')!;

    for (const bad of ['0', '-5', 'abc', '1.23456', '']) {
      setInput(faceInput, bad);
      addBtn.click();
      await settle();
    }

    expect(writes).toHaveLength(0);
    expect(root.querySelector('.add-note-form .error-msg')?.textContent).toBe(
      table.th.faceValueInvalid,
    );
  });

  it('shows server refusal message when POST /api/denomination-notes fails', async () => {
    refusal = { status: 400, code: 'validationFailed', message: 'invalid face value' };
    await mountAndPickKrw();

    const faceInput = root.querySelector<HTMLInputElement>('.add-note-form input')!;
    const addBtn = root.querySelector<HTMLButtonElement>('.add-note-form button')!;
    setInput(faceInput, '1000');
    addBtn.click();
    await settle();

    expect(writes).toHaveLength(1);
    expect(root.querySelector('.add-note-form .error-msg')?.textContent).toBe('invalid face value');
  });
});
