import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewParams } from '../../shared/api/client';
import { en } from '../../shared/i18n/en';
import { th } from '../../shared/i18n/th';
import { screenLookup, setLang } from '../../shared/i18n';
import { OWNER } from './audiences';
import { DocumentsScreen } from './DocumentsScreen';

let dispose: (() => void) | undefined;
let host: HTMLElement;

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

function fakeApi(error: unknown): ApiClient {
  return {
    async view<T>(_name: string, _q: ViewParams = {}): Promise<T> {
      throw error;
    },
    async page() {
      throw new Error('the documents screen pages nothing');
    },
    async act() {
      throw new Error('the documents screen writes nothing');
    },
  } as unknown as ApiClient;
}

async function mountScreen(
  error: unknown,
  audience = OWNER,
): Promise<{ el: HTMLElement; replace: ReturnType<typeof vi.fn> }> {
  const replace = vi.fn();
  host = document.createElement('div');
  document.body.appendChild(host);
  const api = fakeApi(error);
  const props = {
    api,
    audience,
    reference: 'XC-20261006-000002',
    branch: 'BR-2',
    replace,
  };
  dispose = render(() => <DocumentsScreen {...props} />, host);
  await settle();
  return { el: host, replace };
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

describe('DocumentsScreen language switch on refusal line', () => {
  it('row 1: 400 invalidRequest changes between th and en in place', async () => {
    setLang('th');
    const err = new ApiError({ status: 400, code: 'invalidRequest', message: 'คำขอไม่ถูกต้อง' });
    const { el } = await mountScreen(err);

    const refusalEl = el.querySelector('.documents-refusal');
    expect(refusalEl).not.toBeNull();
    expect(refusalEl!.textContent).toBe(screenLookup.th['documents.badBranch']);

    setLang('en');
    await settle();
    expect(refusalEl!.textContent).toBe(screenLookup.en['documents.badBranch']);

    setLang('th');
    await settle();
    expect(refusalEl!.textContent).toBe(screenLookup.th['documents.badBranch']);
  });

  it('row 2: network TypeError in Thai switches to en error.network', async () => {
    setLang('th');
    const err = new TypeError('Failed to fetch');
    const { el } = await mountScreen(err);

    const refusalEl = el.querySelector('.documents-refusal');
    expect(refusalEl).not.toBeNull();
    expect(refusalEl!.textContent).toBe(th['error.network']);

    setLang('en');
    await settle();
    expect(refusalEl!.textContent).toBe(en['error.network']);
  });

  it('row 3: 500 serverError in Thai switches to en error.serverError', async () => {
    setLang('th');
    const err = new ApiError({ status: 500, code: 'serverError', message: 'Internal Server Error' });
    const { el } = await mountScreen(err);

    const refusalEl = el.querySelector('.documents-refusal');
    expect(refusalEl).not.toBeNull();
    expect(refusalEl!.textContent).toBe(th['error.serverError']);

    setLang('en');
    await settle();
    expect(refusalEl!.textContent).toBe(en['error.serverError']);
  });
});
