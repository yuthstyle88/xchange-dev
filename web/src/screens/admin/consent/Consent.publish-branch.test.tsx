import { afterEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
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
  for (let i = 0; i < 10; i++) {
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

function typeBody(root: HTMLElement, text: string): void {
  const textarea = root.querySelector<HTMLTextAreaElement>('.consent-publish-form textarea')!;
  textarea.value = text;
  textarea.dispatchEvent(new Event('input', { bubbles: true }));
  textarea.dispatchEvent(new Event('change', { bubbles: true }));
}

function clickPublish(root: HTMLElement): void {
  const publishBtn = [...root.querySelectorAll<HTMLButtonElement>('.consent-publish-form button')].find(
    (b) => b.textContent?.trim() === table.th.publish,
  )!;
  publishBtn.click();
}

function getPublishButton(root: HTMLElement): HTMLButtonElement {
  return [...root.querySelectorAll<HTMLButtonElement>('.consent-publish-form button')].find(
    (b) => b.textContent?.trim() === table.th.publish,
  )!;
}

function getInlineError(root: HTMLElement): string | null {
  const el = root.querySelector<HTMLDivElement>('.consent-publish-form .error');
  if (!el || !el.textContent?.trim()) return null;
  return el.textContent.trim();
}

function getToastText(): string | null {
  const el = document.querySelector<HTMLDivElement>('.toast');
  if (!el || el.hasAttribute('hidden') || !el.textContent?.trim()) return null;
  return el.textContent.trim();
}

describe('#2306: Admin Consent publish across branch switch', () => {
  it('row 1: Publish on A held, switch to B, answer 4xx refusal -> No inline error and no toast on B', async () => {
    const publishDeferred = deferred<Response>();

    globalThis.fetch = (input, init) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname.includes('views/consent_texts')) {
        const b = url.searchParams.get('branch');
        return Promise.resolve(makeTextsResponse(`TEXT-BRANCH-${b}`, 1));
      }
      if (method === 'POST' && url.pathname === '/api/consent-texts') {
        return publishDeferred.promise;
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root, setBranch } = await mountScreen('A');
    typeBody(root, 'A BODY');
    clickPublish(root);
    await settle();

    // Switch to branch B
    setBranch('B');
    await settle();

    // Release refusal from branch A
    publishDeferred.resolve(
      jsonResponse(409, { error: { code: 'conflict', message: 'A publish refused' } }),
    );
    await settle();

    expect(getInlineError(root)).toBeNull();
    expect(getToastText()).toBeNull();
  });

  it('row 2: Publish on A held, switch to B (B draft typed), answer 200 -> B\'s textarea keeps B DRAFT; B\'s list is shown', async () => {
    const publishDeferred = deferred<Response>();

    globalThis.fetch = (input, init) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname.includes('views/consent_texts')) {
        const b = url.searchParams.get('branch');
        return Promise.resolve(makeTextsResponse(`TEXT-BRANCH-${b}`, 1));
      }
      if (method === 'POST' && url.pathname === '/api/consent-texts') {
        return publishDeferred.promise;
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root, setBranch } = await mountScreen('A');
    typeBody(root, 'A BODY');
    clickPublish(root);
    await settle();

    // Switch to branch B and start typing a draft for B
    setBranch('B');
    await settle();
    typeBody(root, 'B DRAFT');

    // Release success from branch A
    publishDeferred.resolve(
      jsonResponse(200, { purpose: 'MARKETING', lang: 'th', version: 2 }),
    );
    await settle();

    const textarea = root.querySelector<HTMLTextAreaElement>('.consent-publish-form textarea')!;
    expect(textarea.value).toBe('B DRAFT');
    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'TEXT-BRANCH-B',
    );
  });

  it('row 3: A refusal is showing inline on A, then the admin switches to B -> The inline error is cleared on B', async () => {
    globalThis.fetch = (input, init) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname.includes('views/consent_texts')) {
        const b = url.searchParams.get('branch');
        return Promise.resolve(makeTextsResponse(`TEXT-BRANCH-${b}`, 1));
      }
      if (method === 'POST' && url.pathname === '/api/consent-texts') {
        return Promise.resolve(
          jsonResponse(409, { error: { code: 'conflict', message: 'A publish refused' } }),
        );
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root, setBranch } = await mountScreen('A');
    typeBody(root, 'A BODY');
    clickPublish(root);
    await settle();

    // Refusal is visible on A
    expect(getInlineError(root)).toContain('A publish refused');

    // Switch to branch B
    setBranch('B');
    await settle();

    // Inline error must be cleared on B
    expect(getInlineError(root)).toBeNull();
  });

  it('row 4: Publish refused on the same branch -> Inline error and worded toast', async () => {
    globalThis.fetch = (input, init) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname.includes('views/consent_texts')) {
        return Promise.resolve(makeTextsResponse('TEXT-BRANCH-A', 1));
      }
      if (method === 'POST' && url.pathname === '/api/consent-texts') {
        return Promise.resolve(
          jsonResponse(409, { error: { code: 'conflict', message: 'Publish refused on same branch' } }),
        );
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root } = await mountScreen('A');
    typeBody(root, 'A BODY');
    clickPublish(root);
    await settle();

    expect(getInlineError(root)).toContain('Publish refused on same branch');
    expect(getToastText()).toContain('Publish refused on same branch');
  });

  it('row 5: Publish succeeds on the same branch -> Body cleared and list re-read', async () => {
    let readCount = 0;
    globalThis.fetch = (input, init) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname.includes('views/consent_texts')) {
        readCount += 1;
        if (readCount === 1) {
          return Promise.resolve(makeTextsResponse('INITIAL-TEXT', 1));
        }
        return Promise.resolve(makeTextsResponse('UPDATED-PUBLISHED-TEXT', 2));
      }
      if (method === 'POST' && url.pathname === '/api/consent-texts') {
        return Promise.resolve(
          jsonResponse(200, { purpose: 'MARKETING', lang: 'th', version: 2 }),
        );
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root } = await mountScreen('A');
    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'INITIAL-TEXT',
    );

    typeBody(root, 'A BODY');
    clickPublish(root);
    await settle();

    const textarea = root.querySelector<HTMLTextAreaElement>('.consent-publish-form textarea')!;
    expect(textarea.value).toBe('');
    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'UPDATED-PUBLISHED-TEXT',
    );
  });

  it('row 6: Publish on A in flight, switch to B -> The Publish button stays disabled until A\'s answer lands, then is enabled', async () => {
    const publishDeferred = deferred<Response>();

    globalThis.fetch = (input, init) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname.includes('views/consent_texts')) {
        const b = url.searchParams.get('branch');
        return Promise.resolve(makeTextsResponse(`TEXT-BRANCH-${b}`, 1));
      }
      if (method === 'POST' && url.pathname === '/api/consent-texts') {
        return publishDeferred.promise;
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root, setBranch } = await mountScreen('A');
    typeBody(root, 'A BODY');

    const btn = getPublishButton(root);
    expect(btn.disabled).toBe(false);

    clickPublish(root);
    await settle();

    // In flight on A: button disabled
    expect(btn.disabled).toBe(true);

    // Switch to B: button remains disabled while in flight
    setBranch('B');
    await settle();
    expect(btn.disabled).toBe(true);

    // Release publish answer
    publishDeferred.resolve(
      jsonResponse(200, { purpose: 'MARKETING', lang: 'th', version: 2 }),
    );
    await settle();

    // After answer lands: button re-enabled
    expect(btn.disabled).toBe(false);
  });

  it('row 7: Switch A → B → A before the answer lands -> Treated as the same branch: the answer applies', async () => {
    const publishDeferred = deferred<Response>();
    let readCountA = 0;

    globalThis.fetch = (input, init) => {
      const url = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && url.pathname.includes('views/consent_texts')) {
        const b = url.searchParams.get('branch');
        if (b === 'A') {
          readCountA += 1;
          if (readCountA === 1) return Promise.resolve(makeTextsResponse('A-TEXT-V1', 1));
          return Promise.resolve(makeTextsResponse('A-TEXT-V2-PUBLISHED', 2));
        }
        return Promise.resolve(makeTextsResponse('B-TEXT', 1));
      }
      if (method === 'POST' && url.pathname === '/api/consent-texts') {
        return publishDeferred.promise;
      }
      return Promise.resolve(jsonResponse(404, {}));
    };

    const { root, setBranch } = await mountScreen('A');
    typeBody(root, 'A BODY');
    clickPublish(root);
    await settle();

    // Switch A -> B -> A
    setBranch('B');
    await settle();
    setBranch('A');
    await settle();

    // Release publish answer
    publishDeferred.resolve(
      jsonResponse(200, { purpose: 'MARKETING', lang: 'th', version: 2 }),
    );
    await settle();

    const textarea = root.querySelector<HTMLTextAreaElement>('.consent-publish-form textarea')!;
    expect(textarea.value).toBe('');
    expect(root.querySelector('[data-consent-text="MARKETING:th"]')?.textContent).toContain(
      'A-TEXT-V2-PUBLISHED',
    );
  });
});
