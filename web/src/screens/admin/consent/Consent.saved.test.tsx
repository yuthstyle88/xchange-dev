import { afterEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import {
  makeConsentTextsData,
  makeRetentionSetOutput,
} from '../../../shared/testing/fixtures';
import { Toast, toast } from '../parts/toast';
import { Consent } from './Consent';
import * as table from './i18n';

let container: HTMLDivElement | null = null;
let dispose: (() => void) | null = null;
const realFetch = globalThis.fetch;

afterEach(() => {
  dispose?.();
  dispose = null;
  container?.remove();
  container = null;
  globalThis.fetch = realFetch;
  toast('');
  setLang('th');
});

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function mountScreen(): Promise<HTMLDivElement> {
  const [branch] = createSignal('MAIN');
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(
    () => (
      <>
        <Toast />
        <Consent
          branch={branch}
          period={() => ({ from: '2026-10-14', to: '2026-10-15' })}
          shown={() => true}
        />
      </>
    ),
    container,
  );
  await settle();
  return container;
}

describe('R3-7 (#4744): Consent retention save toast', () => {
  it('C1: a valid retention save answers 2xx -> the toast text equals saved (read from the i18n table), not save', async () => {
    globalThis.fetch = (input) => {
      const url = String(input);
      if (url.includes('views/consent_texts')) {
        return Promise.resolve(
          jsonResponse(200, { version: 1, changed: true, data: makeConsentTextsData() }),
        );
      }
      return Promise.resolve(
        jsonResponse(200, makeRetentionSetOutput({ dataType: 'IDENTITY_VALUE', periodDays: 30 })),
      );
    };

    const root = await mountScreen();
    const row = root.querySelector('[data-retention="IDENTITY_VALUE"]')!;
    const input = row.querySelector('input')! as HTMLInputElement;
    input.value = '30';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));

    const saveBtn = row.querySelector('button')! as HTMLButtonElement;
    expect(saveBtn.textContent?.trim()).toBe(table.th.save);
    saveBtn.click();
    await settle();

    const toastText = root.querySelector('.toast')?.textContent?.trim();
    expect(toastText).toBe(table.th.saved);
    expect(toastText).not.toBe(table.th.save);
  });

  it('C2: the save fails -> the worded error toast as today', async () => {
    globalThis.fetch = (input) => {
      const url = String(input);
      if (url.includes('views/consent_texts')) {
        return Promise.resolve(
          jsonResponse(200, { version: 1, changed: true, data: makeConsentTextsData() }),
        );
      }
      return Promise.resolve(
        jsonResponse(500, { error: { code: 'serverError', message: 'internal error' } }),
      );
    };

    const root = await mountScreen();
    const row = root.querySelector('[data-retention="IDENTITY_VALUE"]')!;
    const saveBtn = row.querySelector('button')! as HTMLButtonElement;
    saveBtn.click();
    await settle();

    const toastText = root.querySelector('.toast')?.textContent?.trim();
    expect(toastText).toBe(sharedTh['error.serverError']);
  });
});
