// Issue #2537: Desk customer step — clearing the file input value on "Remove file"
// so choosing the same passport file again fires `change` / `onFile` and stores the passport.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, Show } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { th as sharedTh } from '../../../../shared/i18n/th';
import type { DeskStore } from '../../desk-store';
import { step } from './step';

const REMOVE = 'ลบเอกสาร';
const NO_FILE = 'กรุณาแนบไฟล์สแกนหรือรูปพาสปอร์ตก่อน';

const PATHS = {
  OWNER_DESK: { kind: 'owner', rpc: '/api/rpc', usesDraft: false },
  STAFF_DESK: { kind: 'staff', rpc: '/api/staff/rpc', usesDraft: true },
} as const;

const JPEG = new File([new Uint8Array([0xff, 0xd8, 0xff, 1])], 'passport.jpg', { type: 'image/jpeg' });

type RpcCall = { path: string; action: string; payload: unknown };

function rpcServer(
  answer: (action: string, payload: unknown) => { status: number; body?: unknown; raw?: string; throws?: boolean },
) {
  const calls: RpcCall[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body ?? '{}')) as { action: string; payload: unknown };
    // #3008: the staff draft is v3 draftBegin (POST /api/tx-drafts), not the retired opsBeginTransaction RPC.
    const isDraft = String(input) === '/api/tx-drafts';
    const call = { path: String(input), action: isDraft ? 'draftBegin' : body.action, payload: isDraft ? body : body.payload };
    calls.push(call);
    const a = answer(call.action, call.payload);
    if (a.throws) throw new TypeError('Failed to fetch');
    return new Response(a.raw ?? JSON.stringify(a.body ?? {}), {
      status: a.status,
      headers: { 'content-type': a.raw ? 'text/html' : 'application/json' },
    });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), calls };
}

const OK = (action: string) =>
  action === 'draftBegin'
    ? { status: 200, body: { id: 'draft-9' } }
    : { status: 200, body: { customerId: 'CX-00000042', existing: false } };

function createTestStore(path: (typeof PATHS)[keyof typeof PATHS], api: ApiClient) {
  const [stepIdx, setStepIdx] = createSignal<0 | 1 | 2 | 3>(1);
  const [passport, setPassportS] = createSignal<File | undefined>();
  const [fields, setFields] = createSignal({
    name: '',
    passportNo: '',
    nationality: '',
    birthDate: '',
    passportExpiry: '',
  });
  const [identityChecked, setIC] = createSignal(false);
  const [customerId, setCID] = createSignal('');
  const [staffDraftId, setSDI] = createSignal('');
  const [entry] = createSignal<undefined>(undefined);

  return {
    api,
    path,
    branch: () => 'XC-B01',
    type: () => 'BUY',
    currency: () => 'USD',
    amount: () => '100.50',
    step: stepIdx,
    config: () => ({ shiftOpen: true }),
    passport,
    setPassport: vi.fn((f: File | undefined) => setPassportS(f)),
    fields,
    setField: vi.fn((k: string, v: string, by: 'user' | 'ocr') => {
      setFields({ ...fields(), [k]: v });
      if (by === 'user') setIC(false);
    }),
    editedByUser: () => new Set<string>(),
    identityChecked,
    setIdentityChecked: vi.fn((v: boolean) => setIC(v)),
    customerId,
    setCustomerId: vi.fn((v: string) => setCID(v)),
    staffDraftId,
    setStaffDraftId: vi.fn((v: string) => setSDI(v)),
    entry,
    go: vi.fn((s: 0 | 1 | 2 | 3) => setStepIdx(s)),
  };
}

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

function mountStep(store: ReturnType<typeof createTestStore>) {
  dispose = render(
    () => (
      <Show when={store.step() === 1}>
        {step.component({ store: store as unknown as DeskStore })}
      </Show>
    ),
    host,
  );
}

const q = <T extends Element>(sel: string) => host.querySelector<T>(sel);
const button = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => (b.textContent ?? '').includes(text));

/**
 * Sets `input.files` with `Object.defineProperty` (and syncs jsdom's internal `impl.files`
 * so `input.value` reflects `C:\fakepath\<name>` and `input.value = ''` clears the list).
 * Mirrors browser file-picker semantics: `change` fires only when the chosen path differs
 * from the input's current `value`.
 */
function attach(file: File): HTMLInputElement {
  const input = q<HTMLInputElement>('input[type="file"][name="passport"]');
  expect(input, 'the passport file input').not.toBeNull();
  const prevValue = input!.value;
  const list = [file];
  Object.defineProperty(input!, 'files', { value: list, configurable: true });
  const implSym = Object.getOwnPropertySymbols(input!).find((s) => s.description === 'impl');
  if (implSym) {
    Object.defineProperty((input as unknown as Record<symbol, object>)[implSym]!, 'files', {
      value: list,
      configurable: true,
    });
  }
  if (input!.value !== prevValue) {
    input!.dispatchEvent(new Event('change', { bubbles: true }));
  }
  return input!;
}

function type(name: string, value: string) {
  const el = q<HTMLInputElement>(`input[name="${name}"]`);
  expect(el, `input ${name}`).not.toBeNull();
  el!.value = value;
  el!.dispatchEvent(new Event('input', { bubbles: true }));
  el!.dispatchEvent(new Event('change', { bubbles: true }));
}

function pickNationality(code: string) {
  const sel = q<HTMLSelectElement>('select[name="nationality"]');
  expect(sel, 'the nationality picker').not.toBeNull();
  sel!.value = code;
  sel!.dispatchEvent(new Event('change', { bubbles: true }));
}

function fillFields() {
  type('name', 'JOHN DOE');
  type('passportNo', 'AA1234567');
  pickNationality('USA');
  type('birthDate', '1990-01-01');
  type('passportExpiry', '2030-01-01');
}

const submit = () => q<HTMLButtonElement>('button[type="submit"]')!.click();

describe('steps/customer: remove file clears input value so re-picking the same file works (#2537)', () => {
  it('row 1: Pick file X, press Remove -> Input value is empty and no passport is stored', () => {
    for (const path of Object.values(PATHS)) {
      const s = rpcServer((a) => OK(a));
      const store = createTestStore(path, s.api);
      mountStep(store);

      const input = attach(JPEG);
      expect(input.value).toBe('C:\\fakepath\\passport.jpg');
      expect(store.passport()).toBe(JPEG);

      const removeBtn = button(REMOVE);
      expect(removeBtn, 'the remove file button').toBeDefined();
      removeBtn!.click();

      expect(input.value).toBe('');
      expect(store.passport()).toBeUndefined();
      expect(button(REMOVE)).toBeUndefined();

      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
    }
  });

  it('row 2: Then pick X again -> onFile runs and the passport is stored again', () => {
    for (const path of Object.values(PATHS)) {
      const s = rpcServer((a) => OK(a));
      const store = createTestStore(path, s.api);
      mountStep(store);

      const input = attach(JPEG);
      expect(store.passport()).toBe(JPEG);

      button(REMOVE)!.click();
      expect(store.passport()).toBeUndefined();

      store.setPassport.mockClear();
      attach(JPEG);
      expect(store.setPassport).toHaveBeenCalledTimes(1);
      expect(store.setPassport).toHaveBeenCalledWith(JPEG);
      expect(store.passport()).toBe(JPEG);
      expect(input.value).toBe('C:\\fakepath\\passport.jpg');

      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
    }
  });

  it('row 3: Pick file X, Submit (including after Remove and re-picking X) -> reserves customer and goes to step 2', async () => {
    for (const path of Object.values(PATHS)) {
      const s = rpcServer((a) => OK(a));
      const store = createTestStore(path, s.api);
      mountStep(store);

      fillFields();
      attach(JPEG);
      button(REMOVE)!.click();

      // Submitting while removed refuses with noFile
      submit();
      expect(host.textContent).toContain(NO_FILE);
      expect(s.calls).toHaveLength(0);

      // Re-picking the same file X and submitting succeeds
      attach(JPEG);
      submit();

      await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(2));
      const reserve = s.calls.find((c) => c.action === 'reserveCustomer');
      expect(reserve).toBeDefined();
      expect(store.customerId()).toBe('CX-00000042');

      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
    }
  });

  it('row 4: Leave / skip guards and worded errors remain intact after Remove and re-picking X', async () => {
    // 1. Worded error on 502 after Remove + re-picking X
    const s1 = rpcServer((a) =>
      a === 'reserveCustomer' ? { status: 502, raw: '<html>bad gateway</html>' } : OK(a),
    );
    const store1 = createTestStore(PATHS.OWNER_DESK, s1.api);
    mountStep(store1);

    fillFields();
    attach(JPEG);
    button(REMOVE)!.click();
    attach(JPEG);
    submit();

    await vi.waitFor(() => expect(host.textContent).toContain(sharedTh['error.serverError']));
    expect(store1.go).not.toHaveBeenCalledWith(2);

    dispose?.();
    dispose = undefined;
    host.innerHTML = '';

    // 2. Leave guard when leaving step 1 while reserveCustomer is in flight after Remove + re-picking X
    let resolveRpc!: (res: Response) => void;
    const fetchFn = vi.fn(
      () =>
        new Promise<Response>((r) => {
          resolveRpc = r;
        }),
    );
    const store2 = createTestStore(PATHS.OWNER_DESK, new ApiClient(fetchFn as unknown as typeof fetch));
    mountStep(store2);

    fillFields();
    attach(JPEG);
    button(REMOVE)!.click();
    attach(JPEG);
    submit();

    await vi.waitFor(() => expect(fetchFn).toHaveBeenCalledTimes(1));
    store2.go(0);
    expect(store2.step()).toBe(0);

    resolveRpc(
      new Response(JSON.stringify({ customerId: 'CX-STALE', existing: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );
    await new Promise((r) => setTimeout(r, 10));
    expect(store2.customerId()).toBe('');
    expect(store2.go).not.toHaveBeenCalledWith(2);
  });
});
