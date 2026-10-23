import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { PhoneVerify } from './PhoneVerify';
import { th, en } from './i18n';

type Call = { path: string; key: string | null; body: any };
function fakeServer(confirm: (body: any) => { status: number; body: unknown } = () => ({ status: 200, body: { verified: true, identityId: 'id-1' } })) {
  const calls: Call[] = [];
  let n = 0;
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input).split('?')[0];
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ path, key: new Headers(init?.headers).get('idempotency-key'), body });
    let a: { status: number; body: unknown };
    if (path === '/api/customers/c-1/phone-verifications') a = { status: 200, body: { challengeId: `ch-${++n}`, expiresAt: new Date(Date.now() + 300_000).toISOString() } };
    else if (/^\/api\/customers\/c-1\/phone-verifications\/ch-\d+\/confirm$/.test(path)) a = confirm(body);
    else a = { status: 404, body: { error: { code: 'notFound', message: 'nf' } } };
    return new Response(JSON.stringify(a.body), { status: a.status, headers: { 'content-type': 'application/json' } });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), calls };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
  vi.restoreAllMocks();
});

function mount(server: ReturnType<typeof fakeServer>, onVerified = vi.fn()) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <PhoneVerify api={server.api} branch="MAIN" customerId="c-1" onVerified={onVerified} />, host);
  return onVerified;
}

const el = <T extends Element>(sel: string) => host.querySelector<T>(sel);

function type(sel: string, value: string) {
  const i = el<HTMLInputElement>(sel)!;
  i.value = value;
  i.dispatchEvent(new Event('input', { bubbles: true }));
}

async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

const starts = (s: ReturnType<typeof fakeServer>) => s.calls.filter((c) => c.path === '/api/customers/c-1/phone-verifications');
const confirms = (s: ReturnType<typeof fakeServer>) => s.calls.filter((c) => c.path.endsWith('/confirm'));

describe('PhoneVerify input validation (#4907)', () => {
  it('V1 phone blank -> worded line; no request sent', async () => {
    const server = fakeServer();
    mount(server);
    type('input[name=phone]', '   ');
    el<HTMLButtonElement>('button[name=sendCode]')!.click();
    await flush();

    expect(starts(server)).toHaveLength(0);
    const alert = el('[role=alert]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent?.trim()).toBe(th.phoneRequired);
  });

  it('V2 phone 12ab -> worded line; no request sent', async () => {
    const server = fakeServer();
    mount(server);
    type('input[name=phone]', '12ab');
    el<HTMLButtonElement>('button[name=sendCode]')!.click();
    await flush();

    expect(starts(server)).toHaveLength(0);
    const alert = el('[role=alert]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent?.trim()).toBe(th.invalidPhone);
  });

  it('V3 phone 081-234-5678 -> start sent', async () => {
    const server = fakeServer();
    mount(server);
    type('input[name=phone]', '081-234-5678');
    el<HTMLButtonElement>('button[name=sendCode]')!.click();
    await flush();

    expect(starts(server)).toHaveLength(1);
    expect(starts(server)[0].body).toEqual({ id: 'c-1', branch: 'MAIN', phone: '081-234-5678' });
    expect(el('input[name=code]')).not.toBeNull();
  });

  it('V4 code blank -> worded line; no confirm sent', async () => {
    const server = fakeServer();
    mount(server);
    type('input[name=phone]', '0812345678');
    el<HTMLButtonElement>('button[name=sendCode]')!.click();
    await flush();

    expect(starts(server)).toHaveLength(1);
    type('input[name=code]', '   ');
    el<HTMLButtonElement>('button[name=confirmCode]')!.click();
    await flush();

    expect(confirms(server)).toHaveLength(0);
    const alert = el('[role=alert]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent?.trim()).toBe(th.codeRequired);
  });

  it('superseded refusal displays worded line in th and en', async () => {
    const server = fakeServer(() => ({ status: 409, body: { error: { code: 'superseded', message: 'superseded' } } }));
    mount(server);
    type('input[name=phone]', '0812345678');
    el<HTMLButtonElement>('button[name=sendCode]')!.click();
    await flush();

    type('input[name=code]', '123456');
    el<HTMLButtonElement>('button[name=confirmCode]')!.click();
    await flush();

    expect(el('[role=alert]')?.textContent?.trim()).toBe(th['error.superseded']);

    setLang('en');
    await flush();
    el<HTMLButtonElement>('button[name=confirmCode]')!.click();
    await flush();
    expect(el('[role=alert]')?.textContent?.trim()).toBe(en['error.superseded']);
  });
});
