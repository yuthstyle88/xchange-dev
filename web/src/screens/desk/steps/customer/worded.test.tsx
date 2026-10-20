import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { en as sharedEn } from '../../../../shared/i18n/en';
import { ApiClient } from '../../../../shared/api/client';
import { step } from './step';
import * as texts from './i18n';

const PATH_OWNER = { kind: 'owner', rpc: '/api/rpc', usesDraft: false } as const;
const PATH_STAFF = { kind: 'staff', rpc: '/api/staff/rpc', usesDraft: true } as const;

const DUMMY_FILE = new File([new Uint8Array([1, 2, 3])], 'passport.png', { type: 'image/png' });

function createStubStore(
  path: typeof PATH_OWNER | typeof PATH_STAFF,
  api: ApiClient,
  opts: {
    passport?: File;
    fields?: { name: string; passportNo: string; nationality: string; birthDate: string; passportExpiry: string };
    setPassportHook?: (f: File | undefined) => void;
  } = {},
) {
  const [passport, setPassportSignal] = createSignal<File | undefined>(opts.passport ?? DUMMY_FILE);
  const [fields, setFieldsSignal] = createSignal(
    opts.fields ?? {
      name: 'ALICE SMITH',
      passportNo: 'A12345678',
      nationality: 'USA',
      birthDate: '1995-05-15',
      passportExpiry: '2030-05-15',
    },
  );
  const [identityChecked, setIdentityCheckedSignal] = createSignal(true);
  const [customerId, setCustomerIdSignal] = createSignal('');
  const [staffDraftId, setStaffDraftIdSignal] = createSignal('');

  return {
    api,
    path,
    branch: () => 'XC-B01',
    type: () => 'BUY',
    currency: () => 'USD',
    amount: () => '500.00',
    step: () => 1,
    config: () => ({ shiftOpen: true }),
    passport,
    setPassport: vi.fn((f: File | undefined) => {
      opts.setPassportHook?.(f);
      setPassportSignal(f);
    }),
    fields,
    setField: vi.fn((k: string, v: string) => {
      setFieldsSignal({ ...fields(), [k]: v });
    }),
    editedByUser: () => new Set<string>(),
    identityChecked,
    setIdentityChecked: vi.fn((v: boolean) => setIdentityCheckedSignal(v)),
    customerId,
    setCustomerId: vi.fn((v: string) => setCustomerIdSignal(v)),
    staffDraftId,
    setStaffDraftId: vi.fn((v: string) => setStaffDraftIdSignal(v)),
    entry: () => undefined,
    go: vi.fn(),
  };
}

describe('screens/desk/steps/customer wordedError (WE-33 #1782)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    setLang('th');
  });

  function mount(store: ReturnType<typeof createStubStore>) {
    dispose = render(() => step.component({ store: store as any }), host);
  }

  const submitForm = () => {
    const btn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(btn).not.toBeNull();
    btn!.click();
  };

  const clickSkip = () => {
    const btns = [...host.querySelectorAll('button')];
    const btn = btns.find((b) => b.textContent?.includes(texts.th.skip) || b.textContent?.includes(texts.en.skip));
    expect(btn).toBeDefined();
    btn!.click();
  };

  describe('reserveCustomer submit catch', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      const fetchFn = vi.fn(async () => {
        throw new TypeError('Failed to fetch');
      });
      const api = new ApiClient(fetchFn as unknown as typeof fetch);
      const store = createStubStore(PATH_OWNER, api);
      mount(store);

      submitForm();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedTh['error.network']);
      });

      // Switch language to EN
      setLang('en');
      submitForm();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 server error shows error.serverError', async () => {
      const fetchFn = vi.fn(async () => {
        return new Response(
          JSON.stringify({ error: { code: 'roleNotAllowedHere', message: 'internal server error' } }),
          {
            status: 500,
            headers: { 'content-type': 'application/json' },
          },
        );
      });
      const api = new ApiClient(fetchFn as unknown as typeof fetch);
      const store = createStubStore(PATH_OWNER, api);
      mount(store);

      submitForm();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedTh['error.serverError']);
      });

      setLang('en');
      submitForm();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedEn['error.serverError']);
      });
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const serverMessage = 'Custom server refusal message for passport check';
      const fetchFn = vi.fn(async () => {
        return new Response(JSON.stringify({ error: serverMessage, code: 'unkeyed_customer_refusal' }), {
          status: 400,
          headers: { 'content-type': 'application/json' },
        });
      });
      const api = new ApiClient(fetchFn as unknown as typeof fetch);
      const store = createStubStore(PATH_OWNER, api);
      mount(store);

      submitForm();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(serverMessage);
      });
    });

    it('(edge) refusal with keyed code shows shared translated message', async () => {
      const fetchFn = vi.fn(async () => {
        return new Response(JSON.stringify({ error: { code: 'roleNotAllowedHere', message: 'Role not allowed' } }), {
          status: 403,
          headers: { 'content-type': 'application/json' },
        });
      });
      const api = new ApiClient(fetchFn as unknown as typeof fetch);
      const store = createStubStore(PATH_OWNER, api);
      mount(store);

      submitForm();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedTh['error.roleNotAllowedHere']);
      });
    });

    it('staff desk draftBegin network error shows error.network', async () => {
      const fetchFn = vi.fn(async () => {
        throw new TypeError('Failed to fetch');
      });
      const api = new ApiClient(fetchFn as unknown as typeof fetch);
      const store = createStubStore(PATH_STAFF, api);
      mount(store);

      submitForm();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedTh['error.network']);
      });
    });
  });

  describe('onSkip reserve failure catch (prefix with skipFailed)', () => {
    it('(1) network TypeError on skip shows skipFailed with error.network in TH and EN', async () => {
      const fetchFn = vi.fn(async () => {
        throw new TypeError('Failed to fetch');
      });
      const api = new ApiClient(fetchFn as unknown as typeof fetch);
      const store = createStubStore(PATH_OWNER, api);
      mount(store);

      clickSkip();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedTh['error.network']);
        expect(host.textContent).toContain('ข้ามไม่สำเร็จ:');
      });

      setLang('en');
      clickSkip();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedEn['error.network']);
        expect(host.textContent).toContain('Skip failed:');
      });
    });

    it('(2) 500 server error on skip shows skipFailed with error.serverError', async () => {
      const fetchFn = vi.fn(async () => {
        return new Response(
          JSON.stringify({ error: { code: 'roleNotAllowedHere', message: 'db crashed' } }),
          {
            status: 500,
            headers: { 'content-type': 'application/json' },
          },
        );
      });
      const api = new ApiClient(fetchFn as unknown as typeof fetch);
      const store = createStubStore(PATH_OWNER, api);
      mount(store);

      clickSkip();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedTh['error.serverError']);
        expect(host.textContent).toContain('ข้ามไม่สำเร็จ:');
      });
    });

    it('(3) refusal with unkeyed code on skip shows skipFailed with server message', async () => {
      const serverMessage = 'Customer blacklisted by system';
      const fetchFn = vi.fn(async () => {
        return new Response(JSON.stringify({ error: serverMessage, code: 'unkeyed_skip_refusal' }), {
          status: 400,
          headers: { 'content-type': 'application/json' },
        });
      });
      const api = new ApiClient(fetchFn as unknown as typeof fetch);
      const store = createStubStore(PATH_OWNER, api);
      mount(store);

      clickSkip();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(serverMessage);
        expect(host.textContent).toContain('ข้ามไม่สำเร็จ:');
      });
    });
  });

  describe('onSkip passport generation catch (when noPassportImage or setPassport throws)', () => {
    it('(1) TypeError in passport step shows error.network', async () => {
      const api = new ApiClient(vi.fn() as unknown as typeof fetch);
      const store = createStubStore(PATH_OWNER, api, {
        setPassportHook: () => {
          throw new TypeError('Failed to construct image');
        },
      });
      mount(store);

      clickSkip();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedTh['error.network']);
        expect(host.textContent).toContain('ข้ามไม่สำเร็จ:');
      });

      setLang('en');
      clickSkip();
      await vi.waitFor(() => {
        expect(host.textContent).toContain(sharedEn['error.network']);
        expect(host.textContent).toContain('Skip failed:');
      });
    });
  });
});
