// #4908 (T8): Desk customer step input bounds (name maxLength=100, passportNo maxLength=30,
// birthDate / passportExpiry min="1920-04-02", and native/submit constraint checks).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import type { DeskStore, PassportFields } from '../../desk-store';
import { step } from './step';

const PATH_OWNER = { kind: 'owner', rpc: '/api/rpc', usesDraft: false } as const;
const JPEG = new File([new Uint8Array([0xff, 0xd8, 0xff, 1])], 'passport.jpg', { type: 'image/jpeg' });

type RpcCall = { path: string; action: string; payload: Record<string, unknown> };

function rpcServer() {
  const calls: RpcCall[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body ?? '{}')) as { action: string; payload: Record<string, unknown> };
    calls.push({ path: String(input), action: body.action, payload: body.payload });
    return new Response(JSON.stringify({ customerId: 'CX-00000042', existing: false }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), calls };
}

function stubStore(api: ApiClient) {
  const [passport, setPassportS] = createSignal<File | undefined>();
  const [fields, setFields] = createSignal<PassportFields>({
    name: '',
    passportNo: '',
    nationality: '',
    birthDate: '',
    passportExpiry: '',
  });
  const [identityChecked, setIC] = createSignal(false);
  const [customerId, setCID] = createSignal('');
  const [staffDraftId, setSDI] = createSignal('');

  const store = {
    api,
    path: PATH_OWNER,
    branch: () => 'XC-B01',
    type: () => 'BUY' as const,
    currency: () => 'USD',
    amount: () => '100.50',
    step: () => 1 as const,
    config: () => ({ shiftOpen: true }),
    passport,
    setPassport: vi.fn((f: File | undefined) => setPassportS(f)),
    fields,
    setField: vi.fn((k: keyof PassportFields, v: string, by: 'user' | 'ocr') => {
      setFields({ ...fields(), [k]: v });
      if (by === 'user') setIC(false);
    }),
    editedByUser: () => new Set<keyof PassportFields>(),
    identityChecked,
    setIdentityChecked: vi.fn((v: boolean) => setIC(v)),
    customerId,
    setCustomerId: vi.fn((v: string) => setCID(v)),
    staffDraftId,
    setStaffDraftId: vi.fn((v: string) => setSDI(v)),
    entry: () => undefined,
    go: vi.fn(),
  };
  return store;
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

function mount(store: ReturnType<typeof stubStore>) {
  dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);
}

const q = <T extends Element>(sel: string) => host.querySelector<T>(sel);

function attach(file: File) {
  const input = q<HTMLInputElement>('input[type="file"][name="passport"]')!;
  Object.defineProperty(input, 'files', { value: [file], configurable: true });
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function type(name: string, value: string): HTMLInputElement {
  const el = q<HTMLInputElement>(`input[name="${name}"]`)!;
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return el;
}

function pickNationality(code: string) {
  const sel = q<HTMLSelectElement>('select[name="nationality"]')!;
  sel.value = code;
  sel.dispatchEvent(new Event('change', { bubbles: true }));
}

const submit = () => q<HTMLButtonElement>('button[type="submit"]')!.click();

describe('steps/customer bounds (#4908)', () => {
  it('C1 name: 101 characters -> input holds 100', () => {
    const s = rpcServer();
    const store = stubStore(s.api);
    mount(store);

    const input = type('name', 'A'.repeat(101));
    expect(input.maxLength).toBe(100);
    expect(input.value).toHaveLength(100);
    expect(store.fields().name).toHaveLength(100);
  });

  it('C2 passport no: 31 characters -> input holds 30', () => {
    const s = rpcServer();
    const store = stubStore(s.api);
    mount(store);

    const input = type('passportNo', 'B'.repeat(31));
    expect(input.maxLength).toBe(30);
    expect(input.value).toHaveLength(30);
    expect(store.fields().passportNo).toHaveLength(30);
  });

  it('C3 birth date: 1900-01-01 -> refused at the field; no request', async () => {
    const s = rpcServer();
    const store = stubStore(s.api);
    mount(store);

    attach(JPEG);
    type('name', 'JOHN DOE');
    type('passportNo', 'AA1234567');
    pickNationality('USA');
    const birthInput = type('birthDate', '1900-01-01');
    const expiryInput = q<HTMLInputElement>('input[name="passportExpiry"]')!;

    expect(birthInput.min).toBe('1920-04-02');
    expect(expiryInput.min).toBe('1920-04-02');
    expect(birthInput.validity.rangeUnderflow).toBe(true);
    expect(birthInput.validity.valid).toBe(false);

    const form = q<HTMLFormElement>('form')!;
    form.noValidate = true;
    submit();
    await Promise.resolve();

    expect(s.calls).toHaveLength(0);
    expect(store.go).not.toHaveBeenCalled();
    expect(birthInput.closest('label')?.textContent).toContain('1920-04-02');
  });

  it('C4 all valid -> request sent as today', async () => {
    const s = rpcServer();
    const store = stubStore(s.api);
    mount(store);

    attach(JPEG);
    type('name', 'JOHN DOE');
    type('passportNo', 'AA1234567');
    pickNationality('USA');
    type('birthDate', '1990-01-01');
    type('passportExpiry', '2030-01-01');

    submit();
    await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(2));

    expect(s.calls).toHaveLength(1);
    expect(s.calls[0]).toEqual({
      path: '/api/rpc',
      action: 'reserveCustomer',
      payload: {
        name: 'JOHN DOE',
        passportNo: 'AA1234567',
        nationality: 'USA',
        birthDate: '1990-01-01',
        passportExpiry: '2030-01-01',
        mode: 'LIVE',
      },
    });
  });
});
