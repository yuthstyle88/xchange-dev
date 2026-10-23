// SF-5 up-front tests (docs/design/web-silent-failures.md §6, owner 2026-10-22 "A"): sign-up is blocked until the TERMS
// text has loaded and is shown. The submit button is disabled unless TermsText reports `ready`, and the screen never
// posts `/api/app/register` without a shown TERMS text. The implementer makes these pass and must not edit them.
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { SignUp } from './SignUp';

type Reply = { status: number; body: unknown } | 'network';

const TERMS = { purpose: 'TERMS', version: 3, body: 'Terms body, version three.' };

/** A fake server for the screen. `texts` may be a function, so a test can hold the answer or change it on retry. */
function server(over: { texts?: Reply | (() => Reply | Promise<Reply>) } = {}) {
  const registers: unknown[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const u = new URL(String(input), 'http://localhost');
    if (u.pathname === '/api/app/register') {
      registers.push(init?.body ? JSON.parse(String(init.body)) : undefined);
      return new Response(JSON.stringify({ ok: true }), { status: 200, headers: { 'content-type': 'application/json' } });
    }
    let r: Reply;
    if (u.pathname === '/api/public/host') r = { status: 200, body: { org: 'XC' } };
    else if (u.pathname === '/api/public/orgs/XC/consent_texts') {
      const t = over.texts ?? { status: 200, body: [TERMS] };
      r = typeof t === 'function' ? await t() : t;
    } else return new Response('{}', { status: 404 });
    if (r === 'network') throw new TypeError('Failed to fetch');
    return new Response(JSON.stringify(r.body), { status: r.status, headers: { 'content-type': 'application/json' } });
  });
  return { fetchFn: fetchFn as unknown as typeof fetch, registers };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let navigate: Mock<(url: string) => void>;

beforeEach(() => {
  setAudience('customer');
  setLang('en');
  host = document.createElement('div');
  document.body.appendChild(host);
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang('th');
});

function mount(fetchFn: typeof fetch): void {
  navigate = vi.fn<(url: string) => void>();
  dispose = render(() => <SignUp navigate={navigate} fetch={fetchFn} />, host);
}

const submitBtn = () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
const q = (id: string) => host.querySelector<HTMLElement>(`[data-testid="${id}"]`);

function fillValid(): void {
  for (const [name, val] of [
    ['displayName', 'Somchai'],
    ['identifier', '0812345678'],
    ['password', 'pass-1234'],
  ] as const) {
    const el = host.querySelector<HTMLInputElement>(`input[name="${name}"]`)!;
    el.value = val;
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }
}

/** Every way a person can try to submit: the button, and the form's own submit (Enter in a field). */
async function trySubmit(): Promise<void> {
  submitBtn().click();
  host.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await flush();
}

describe('SF-5: sign-up waits for the TERMS text', () => {
  it('loading: submit is disabled while the texts have not answered, and enabled once the text is shown', async () => {
    let release!: (r: Reply) => void;
    const s = server({ texts: () => new Promise<Reply>((res) => (release = res)) });
    mount(s.fetchFn);
    await flush();
    expect(submitBtn().disabled).toBe(true);
    await trySubmit();
    expect(s.registers).toHaveLength(0);

    release({ status: 200, body: [TERMS] });
    await flush();
    expect(q('terms-body')?.textContent).toContain('Terms body, version three.');
    expect(submitBtn().disabled).toBe(false);
  });

  it('guard: ready ⇒ a click posts /api/app/register once and navigates', async () => {
    const s = server();
    mount(s.fetchFn);
    await flush();
    expect(q('terms-body')).not.toBeNull();
    fillValid();
    submitBtn().click();
    await flush();
    expect(s.registers).toHaveLength(1);
    expect(navigate).toHaveBeenCalledWith('/app');
  });

  it.each<[string, Reply]>([
    ['no text in this language ([])', { status: 200, body: [] }],
    ['only a MARKETING text', { status: 200, body: [{ purpose: 'MARKETING', version: 1, body: 'Offers.' }] }],
  ])('missing (%s): submit is disabled, and trying to submit sends no register', async (_why, texts) => {
    const s = server({ texts });
    mount(s.fetchFn);
    await flush();
    expect(q('terms-missing')).not.toBeNull();
    expect(submitBtn().disabled).toBe(true);
    await trySubmit();
    expect(s.registers).toHaveLength(0);
    expect(navigate).not.toHaveBeenCalled();
  });

  it.each<[string, Reply]>([
    ['the texts read fails (503)', { status: 503, body: {} }],
    ['the network is down', 'network'],
    ['the answer is not an array', { status: 200, body: {} }],
  ])('failed (%s): submit is disabled, and trying to submit sends no register', async (_why, texts) => {
    const s = server({ texts });
    mount(s.fetchFn);
    await flush();
    expect(q('terms-failed')).not.toBeNull();
    expect(submitBtn().disabled).toBe(true);
    await trySubmit();
    expect(s.registers).toHaveLength(0);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('failed, then retry: the retry button does not submit the form; a good answer enables submit', async () => {
    let answer: Reply = { status: 503, body: {} };
    const s = server({ texts: () => answer });
    mount(s.fetchFn);
    await flush();
    expect(submitBtn().disabled).toBe(true);

    answer = { status: 200, body: [TERMS] };
    expect(q('terms-retry'), 'retry button').not.toBeNull();
    q('terms-retry')!.click();
    await flush();
    expect(s.registers).toHaveLength(0);
    expect(q('terms-body')?.textContent).toContain('Terms body, version three.');
    expect(submitBtn().disabled).toBe(false);

    fillValid();
    submitBtn().click();
    await flush();
    expect(s.registers).toHaveLength(1);
  });

  it('failed, then retry that fails again: submit stays disabled', async () => {
    const s = server({ texts: 'network' });
    mount(s.fetchFn);
    await flush();
    expect(q('terms-retry'), 'retry button').not.toBeNull();
    q('terms-retry')!.click();
    await flush();
    expect(submitBtn().disabled).toBe(true);
    await trySubmit();
    expect(s.registers).toHaveLength(0);
  });
});
