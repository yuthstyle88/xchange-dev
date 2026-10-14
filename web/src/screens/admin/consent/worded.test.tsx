import { afterEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en } from '../../../shared/i18n/en';
import { th } from '../../../shared/i18n/th';
import { Toast } from '../parts/toast';
import { Consent } from './Consent';

let container: HTMLDivElement | null = null;
let dispose: (() => void) | null = null;

const realFetch = globalThis.fetch;

afterEach(() => {
  if (dispose) {
    dispose();
    dispose = null;
  }
  if (container) {
    container.remove();
    container = null;
  }
  globalThis.fetch = realFetch;
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

function typeInto(el: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

async function mountScreen(initialBranch = 'MAIN'): Promise<{
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

describe('WE-12: screens/admin/consent/ failed call handling (docs/design/web-worded-error.md §4.2)', () => {
  describe('loadTexts (reading consent_texts view)', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      globalThis.fetch = () => Promise.reject(new TypeError('Failed to fetch'));
      const { root } = await mountScreen();
      const toastEl = root.querySelector('.toast');
      expect(toastEl?.textContent).toContain(th['error.network']);

      // after setLang('en')
      dispose?.();
      dispose = null;
      container?.remove();
      container = null;

      setLang('en');
      const { root: rootEn } = await mountScreen();
      const toastEn = rootEn.querySelector('.toast');
      expect(toastEn?.textContent).toContain(en['error.network']);
    });

    it('(2) a 500 refusal shows th[\'error.serverError\']', async () => {
      globalThis.fetch = () =>
        Promise.resolve(
          jsonResponse(500, {
            error: { code: 'roleNotAllowedHere', message: 'fatal: pool exhausted' },
          }),
        );
      const { root } = await mountScreen();
      const toastEl = root.querySelector('.toast');
      expect(toastEl?.textContent).toContain(th['error.serverError']);
    });

    it('(3) an unknown code with a server message shows that message', async () => {
      const customMessage = 'ไม่สามารถอ่านข้อความความยินยอมได้ในขณะนี้';
      globalThis.fetch = () =>
        Promise.resolve(
          jsonResponse(400, {
            error: { code: 'consentReadRefused', message: customMessage },
          }),
        );
      const { root } = await mountScreen();
      const toastEl = root.querySelector('.toast');
      expect(toastEl?.textContent).toContain(customMessage);
    });

    it('(4) a non-5xx keyed code (403 roleNotAllowedHere) shows th[\'error.roleNotAllowedHere\']', async () => {
      globalThis.fetch = () =>
        Promise.resolve(
          jsonResponse(403, {
            error: { code: 'roleNotAllowedHere', message: 'raw: not allowed to read consent' },
          }),
        );
      const { root } = await mountScreen();
      const toastEl = root.querySelector('.toast');
      expect(toastEl?.textContent).toContain(th['error.roleNotAllowedHere']);
      expect(toastEl?.textContent).not.toContain('raw: not allowed to read consent');
    });

    it('clears previous branch consent texts when branch switches and read fails with 500', async () => {
      globalThis.fetch = (input) => {
        const url = new URL(String(input), 'http://localhost');
        if (url.pathname.includes('views/consent_texts')) {
          const branch = url.searchParams.get('branch');
          if (branch === 'MAIN') {
            return Promise.resolve(
              jsonResponse(200, {
                version: 1,
                changed: true,
                data: {
                  texts: [
                    {
                      purpose: 'TERMS',
                      lang: 'th',
                      version: 1,
                      body: 'MAIN-ONLY-TEXT',
                      updatedAt: '2026-10-14T00:00:00Z',
                      updatedBy: 'admin',
                    },
                  ],
                },
              }),
            );
          }
          return Promise.resolve(
            jsonResponse(500, {
              error: { code: 'roleNotAllowedHere', message: 'fatal on B02' },
            }),
          );
        }
        return Promise.resolve(jsonResponse(200, { version: 1, changed: false }));
      };

      const { root, setBranch } = await mountScreen('MAIN');
      expect(root.textContent).toContain('MAIN-ONLY-TEXT');
      expect(root.querySelectorAll('[data-consent-text]').length).toBe(1);

      setBranch('B02');
      await settle();

      expect(root.textContent).not.toContain('MAIN-ONLY-TEXT');
      expect(root.querySelectorAll('[data-consent-text]').length).toBe(0);
      const toastEl = root.querySelector('.toast');
      expect(toastEl?.textContent).toContain(th['error.serverError']);
    });
  });

  describe('handlePublish (POST /api/consent-texts)', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      globalThis.fetch = (input) => {
        const url = String(input);
        if (url.includes('views/consent_texts')) {
          return Promise.resolve(jsonResponse(200, { data: { texts: [] } }));
        }
        return Promise.reject(new TypeError('Failed to fetch'));
      };
      const { root } = await mountScreen();
      const textarea = root.querySelector('textarea')!;
      typeInto(textarea, 'ยอมรับข้อกำหนด');
      const publishBtn = root.querySelector('button.primary')! as HTMLButtonElement;
      publishBtn.click();
      await settle();

      const errorEl = root.querySelector('.consent-section .error');
      expect(errorEl?.textContent).toContain(th['error.network']);

      // after setLang('en')
      setLang('en');
      await settle();
      expect(errorEl?.textContent).toContain(en['error.network']);
    });

    it('(2) a 500 refusal shows th[\'error.serverError\']', async () => {
      globalThis.fetch = (input) => {
        const url = String(input);
        if (url.includes('views/consent_texts')) {
          return Promise.resolve(jsonResponse(200, { data: { texts: [] } }));
        }
        return Promise.resolve(
          jsonResponse(500, {
            error: { code: 'roleNotAllowedHere', message: 'internal: crash' },
          }),
        );
      };
      const { root } = await mountScreen();
      const textarea = root.querySelector('textarea')!;
      typeInto(textarea, 'ยอมรับข้อกำหนด');
      const publishBtn = root.querySelector('button.primary')! as HTMLButtonElement;
      publishBtn.click();
      await settle();

      const errorEl = root.querySelector('.consent-section .error');
      expect(errorEl?.textContent).toContain(th['error.serverError']);
    });

    it('(3) an unknown code with a server message shows that message', async () => {
      const customMessage = 'เวอร์ชันของข้อกำหนดซ้ำกับที่มีอยู่';
      globalThis.fetch = (input) => {
        const url = String(input);
        if (url.includes('views/consent_texts')) {
          return Promise.resolve(jsonResponse(200, { data: { texts: [] } }));
        }
        return Promise.resolve(
          jsonResponse(400, {
            error: { code: 'versionDuplicate', message: customMessage },
          }),
        );
      };
      const { root } = await mountScreen();
      const textarea = root.querySelector('textarea')!;
      typeInto(textarea, 'ยอมรับข้อกำหนด');
      const publishBtn = root.querySelector('button.primary')! as HTMLButtonElement;
      publishBtn.click();
      await settle();

      const errorEl = root.querySelector('.consent-section .error');
      expect(errorEl?.textContent).toContain(customMessage);
    });

    it('(4) a non-5xx keyed code (403 roleNotAllowedHere) shows th[\'error.roleNotAllowedHere\']', async () => {
      globalThis.fetch = (input) => {
        const url = String(input);
        if (url.includes('views/consent_texts')) {
          return Promise.resolve(jsonResponse(200, { data: { texts: [] } }));
        }
        return Promise.resolve(
          jsonResponse(403, {
            error: { code: 'roleNotAllowedHere', message: 'raw: not allowed to publish consent' },
          }),
        );
      };
      const { root } = await mountScreen();
      const textarea = root.querySelector('textarea')!;
      typeInto(textarea, 'ยอมรับข้อกำหนด');
      const publishBtn = root.querySelector('button.primary')! as HTMLButtonElement;
      publishBtn.click();
      await settle();

      const errorEl = root.querySelector('.consent-section .error');
      expect(errorEl?.textContent).toContain(th['error.roleNotAllowedHere']);
      expect(errorEl?.textContent).not.toContain('raw: not allowed to publish consent');
    });
  });

  describe('RetentionRow (PUT /api/retention-policies/:type)', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      globalThis.fetch = (input) => {
        const url = String(input);
        if (url.includes('views/consent_texts')) {
          return Promise.resolve(jsonResponse(200, { data: { texts: [] } }));
        }
        return Promise.reject(new TypeError('Failed to fetch'));
      };
      const { root } = await mountScreen();
      const row = root.querySelector('[data-retention="IDENTITY_VALUE"]')!;
      const saveBtn = row.querySelector('button')! as HTMLButtonElement;
      saveBtn.click();
      await settle();

      const errorEl = row.querySelector('.error');
      expect(errorEl?.textContent).toContain(th['error.network']);

      // after setLang('en')
      setLang('en');
      await settle();
      expect(errorEl?.textContent).toContain(en['error.network']);
    });

    it('(2) a 500 refusal shows th[\'error.serverError\']', async () => {
      globalThis.fetch = (input) => {
        const url = String(input);
        if (url.includes('views/consent_texts')) {
          return Promise.resolve(jsonResponse(200, { data: { texts: [] } }));
        }
        return Promise.resolve(
          jsonResponse(500, {
            error: { code: 'roleNotAllowedHere', message: 'db lock failure' },
          }),
        );
      };
      const { root } = await mountScreen();
      const row = root.querySelector('[data-retention="IDENTITY_VALUE"]')!;
      const saveBtn = row.querySelector('button')! as HTMLButtonElement;
      saveBtn.click();
      await settle();

      const errorEl = row.querySelector('.error');
      expect(errorEl?.textContent).toContain(th['error.serverError']);
    });

    it('(3) an unknown code with a server message shows that message', async () => {
      const customMessage = 'ไม่อนุญาตให้แก้ไขนโยบายการจัดเก็บนี้';
      globalThis.fetch = (input) => {
        const url = String(input);
        if (url.includes('views/consent_texts')) {
          return Promise.resolve(jsonResponse(200, { data: { texts: [] } }));
        }
        return Promise.resolve(
          jsonResponse(400, {
            error: { code: 'retentionPolicyLocked', message: customMessage },
          }),
        );
      };
      const { root } = await mountScreen();
      const row = root.querySelector('[data-retention="IDENTITY_VALUE"]')!;
      const saveBtn = row.querySelector('button')! as HTMLButtonElement;
      saveBtn.click();
      await settle();

      const errorEl = row.querySelector('.error');
      expect(errorEl?.textContent).toContain(customMessage);
    });

    it('(4) a non-5xx keyed code (403 roleNotAllowedHere) shows th[\'error.roleNotAllowedHere\']', async () => {
      globalThis.fetch = (input) => {
        const url = String(input);
        if (url.includes('views/consent_texts')) {
          return Promise.resolve(jsonResponse(200, { data: { texts: [] } }));
        }
        return Promise.resolve(
          jsonResponse(403, {
            error: { code: 'roleNotAllowedHere', message: 'raw: not allowed to set retention' },
          }),
        );
      };
      const { root } = await mountScreen();
      const row = root.querySelector('[data-retention="IDENTITY_VALUE"]')!;
      const saveBtn = row.querySelector('button')! as HTMLButtonElement;
      saveBtn.click();
      await settle();

      const errorEl = row.querySelector('.error');
      expect(errorEl?.textContent).toContain(th['error.roleNotAllowedHere']);
      expect(errorEl?.textContent).not.toContain('raw: not allowed to set retention');
    });
  });
});
