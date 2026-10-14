import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en } from '../../../shared/i18n/en';
import { th } from '../../../shared/i18n/th';
import { RatesCard } from './RatesCard';
import type { StaffCtx } from '../cards';

type Answer =
  | { kind: 'ok'; data: unknown }
  | { kind: 'status'; status: number; body: string }
  | { kind: 'network' };

let reads = 0;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mockCtx(): StaffCtx {
  return {
    me: {
      userId: 'test-user-id',
      name: 'Tester',
      roles: [],
      branchId: 'MAIN',
      branchName: 'Main Branch',
      permissions: ['CompatRead'],
    },
    floor: undefined,
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

function stubAnswers(answers: Answer[]) {
  reads = 0;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (!url.startsWith('/api/v3/views/staff_published_rates')) {
        return new Response(JSON.stringify({ error: { code: 'notFound', message: 'unexpected request' } }), {
          status: 404,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      const a = answers[Math.min(reads, answers.length - 1)]!;
      reads++;
      if (a.kind === 'network') throw new TypeError('Failed to fetch');
      if (a.kind === 'status') {
        return new Response(a.body, { status: a.status, headers: { 'Content-Type': 'application/json' } });
      }
      return new Response(JSON.stringify({ version: 1, changed: true, data: a.data }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(answers: Answer[]) {
  stubAnswers(answers);
  dispose = render(() => <RatesCard ctx={mockCtx()} />, host);
  await vi.waitFor(() => expect(reads).toBeGreaterThan(0));
  await settle();
  return host;
}

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('WE-42 screens/staff/rates/ worded errors (#1791)', () => {
  it('(1) a network TypeError shows th["error.network"], and after setLang("en") shows en["error.network"]', async () => {
    setLang('th');
    const el = await mount([{ kind: 'network' }]);
    const alert = el.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toBe(th['error.network']);

    setLang('en');
    el.querySelector('button')?.click();
    await vi.waitFor(() => expect(reads).toBe(2));
    await settle();
    expect(alert!.textContent).toBe(en['error.network']);
  });

  it('(2) a 500 refusal shows th["error.serverError"]', async () => {
    setLang('th');
    const el = await mount([
      {
        kind: 'status',
        status: 500,
        body: JSON.stringify({ error: { code: 'custom500Code', message: 'internal db timeout' } }),
      },
    ]);
    const alert = el.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toBe(th['error.serverError']);
  });

  it('(3) a refusal with a code that has no shared key and a server message shows that message', async () => {
    setLang('th');
    const sentence = 'ไม่สามารถอ่านอัตราแลกเปลี่ยนของสาขานี้ได้ในขณะนี้';
    const el = await mount([
      {
        kind: 'status',
        status: 400,
        body: JSON.stringify({ error: { code: 'unkeyedSpecificCode', message: sentence } }),
      },
    ]);
    const alert = el.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toBe(sentence);
  });
});
