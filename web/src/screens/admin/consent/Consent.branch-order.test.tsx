import { afterEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { Toast, toast } from '../parts/toast';
import { Consent } from './Consent';
import * as table from './i18n';

let container: HTMLDivElement | null = null;
let dispose: (() => void) | null = null;

const realFetch = globalThis.fetch;

afterEach(async () => {
  if (dispose) {
    dispose();
    dispose = null;
  }
  if (container) {
    container.remove();
    container = null;
  }
  toast('');
  globalThis.fetch = realFetch;
  setLang('th');
  await settle();
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

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function makeTextsResponse(bodyText: string, version = 1): Response {
  return jsonResponse(200, {
    version,
    changed: true,
    data: {
      texts: [
        {
          purpose: 'MARKETING',
          lang: 'th',
          version,
          body: bodyText,
          updatedAt: '2026-10-16T00:00:00Z',
          updatedBy: 'admin',
        },
      ],
    },
  });
}

async function mountScreen(initialBranch = 'A'): Promise<{
  root: HTMLDivElement;
  setBranch: (b: string) => void;
}> {
  const [branch, setBranch] = createSignal(initialBranch);
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
  return { root: container, setBranch };
}

describe('Admin Consent texts branch switch and request ordering (#2290)', () => {
  it('row 1: Branch A to B, A answers last -> B\'s texts are shown', async () => {
    const readA = deferred<Response>();
    const readB = deferred<Response>();

    globalThis.fetch = (input) => {
      const url = new URL(String(input), 'http://localhost');
      if (url.pathname.includes('views/consent_texts')) {
        const branch = url.searchParams.get('branch');
        if (branch === 'A') return readA.promise;
        if (branch === 'B') return readB.promise;
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root, setBranch } = await mountScreen('A');
    setBranch('B');
    await settle();

    readB.resolve(makeTextsResponse('TEXT-FROM-BRANCH-B', 2));
    await settle();
    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'TEXT-FROM-BRANCH-B',
    );

    readA.resolve(makeTextsResponse('STALE-TEXT-FROM-BRANCH-A', 1));
    await settle();

    const item = root.querySelector('[data-consent-text="MARKETING:th"]');
    expect(item?.textContent).toContain('TEXT-FROM-BRANCH-B');
    expect(item?.textContent).not.toContain('STALE-TEXT-FROM-BRANCH-A');
  });

  it('row 2: An older read fails after a newer success -> No toast; B\'s texts stay', async () => {
    const readA = deferred<Response>();
    const readB = deferred<Response>();

    globalThis.fetch = (input) => {
      const url = new URL(String(input), 'http://localhost');
      if (url.pathname.includes('views/consent_texts')) {
        const branch = url.searchParams.get('branch');
        if (branch === 'A') return readA.promise;
        if (branch === 'B') return readB.promise;
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root, setBranch } = await mountScreen('A');
    setBranch('B');
    await settle();

    readB.resolve(makeTextsResponse('TEXT-FROM-BRANCH-B', 2));
    await settle();
    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'TEXT-FROM-BRANCH-B',
    );

    readA.resolve(
      jsonResponse(500, {
        error: { code: 'serverError', message: 'late failure from branch A' },
      }),
    );
    await settle();

    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'TEXT-FROM-BRANCH-B',
    );
    expect(root.querySelector<HTMLElement>('.toast')?.hidden).toBe(true);
    expect(root.querySelector('.toast')?.textContent).toBe('');
  });

  it('row 3: B answers with no texts -> Empty list, not A\'s texts', async () => {
    globalThis.fetch = (input) => {
      const url = new URL(String(input), 'http://localhost');
      if (url.pathname.includes('views/consent_texts')) {
        const branch = url.searchParams.get('branch');
        if (branch === 'A') {
          return Promise.resolve(makeTextsResponse('TEXT-FROM-BRANCH-A', 1));
        }
        if (branch === 'B') {
          return Promise.resolve(
            jsonResponse(200, {
              version: 1,
              changed: false,
            }),
          );
        }
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root, setBranch } = await mountScreen('A');
    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'TEXT-FROM-BRANCH-A',
    );

    setBranch('B');
    await settle();

    expect(root.querySelectorAll('[data-consent-text]').length).toBe(0);
    expect(root.textContent).not.toContain('TEXT-FROM-BRANCH-A');
  });

  it('row 4: A read fails -> List empty and the worded error toast (kept), while a stale failure does not overwrite newer branch texts', async () => {
    const readB = deferred<Response>();

    globalThis.fetch = (input) => {
      const url = new URL(String(input), 'http://localhost');
      if (url.pathname.includes('views/consent_texts')) {
        const branch = url.searchParams.get('branch');
        if (branch === 'A') {
          return Promise.resolve(
            jsonResponse(500, {
              error: { code: 'serverError', message: 'branch A failed' },
            }),
          );
        }
        if (branch === 'B') return readB.promise;
        if (branch === 'C') {
          return Promise.resolve(makeTextsResponse('TEXT-FROM-BRANCH-C', 3));
        }
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root, setBranch } = await mountScreen('A');
    expect(root.querySelectorAll('[data-consent-text]').length).toBe(0);
    expect(root.querySelector('.toast')?.textContent).toContain(th['error.serverError']);

    setBranch('B');
    await settle();
    setBranch('C');
    await settle();
    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'TEXT-FROM-BRANCH-C',
    );

    readB.reject(new TypeError('Failed to fetch'));
    await settle();
    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'TEXT-FROM-BRANCH-C',
    );
  });

  it('edge case: Publish succeeds, then a re-read goes through the same guard so an older slow read cannot overwrite it', async () => {
    const initialSlowRead = deferred<Response>();
    let readCount = 0;

    globalThis.fetch = (input, init) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname.includes('views/consent_texts')) {
        readCount += 1;
        if (readCount === 1) return initialSlowRead.promise;
        return Promise.resolve(makeTextsResponse('PUBLISHED-FRESH-TEXT', 2));
      }
      if (method === 'POST' && url.pathname === '/api/consent-texts') {
        return Promise.resolve(jsonResponse(200, { purpose: 'MARKETING', lang: 'th', version: 2 }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root } = await mountScreen('A');

    const textarea = root.querySelector<HTMLTextAreaElement>('.consent-publish-form textarea')!;
    textarea.value = 'PUBLISHED-FRESH-TEXT';
    textarea.dispatchEvent(new Event('input', { bubbles: true }));
    textarea.dispatchEvent(new Event('change', { bubbles: true }));

    const publishBtn = [...root.querySelectorAll<HTMLButtonElement>('.consent-publish-form button')].find(
      (b) => b.textContent?.trim() === table.th.publish,
    )!;
    publishBtn.click();
    await settle();

    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'PUBLISHED-FRESH-TEXT',
    );

    initialSlowRead.resolve(makeTextsResponse('STALE-INITIAL-READ', 1));
    await settle();

    const item = root.querySelector('[data-consent-text="MARKETING:th"]');
    expect(item?.textContent).toContain('PUBLISHED-FRESH-TEXT');
    expect(item?.textContent).not.toContain('STALE-INITIAL-READ');
  });
});
