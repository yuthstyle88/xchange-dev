// SF-5 up-front tests (docs/design/web-silent-failures.md §6, owner 2026-10-22 "A"): TermsText reports where the TERMS
// text is (`loading`, `ready`, `missing`, `failed`) through `onState`, and never shows the static "you accept the terms"
// line in place of a text it could not show. The implementer makes these pass and must not edit them.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import * as table from './i18n';
import { TermsText, type TermsState } from './TermsText';

type Reply = { status: number; body: unknown } | 'network';

const TERMS = { purpose: 'TERMS', version: 2, body: 'Terms body, version two.' };
const MARKETING = { purpose: 'MARKETING', version: 1, body: 'We may send you offers.' };

/** A fake server. `texts` may be a function, so a test can change the answer between reads (retry). */
function server(over: { host?: Reply; texts?: Reply | (() => Reply | Promise<Reply>) } = {}) {
  const urls: URL[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL) => {
    const u = new URL(String(input), 'http://localhost');
    urls.push(u);
    let r: Reply;
    if (u.pathname === '/api/public/host') r = over.host ?? { status: 200, body: { org: 'XC' } };
    else if (u.pathname === '/api/public/orgs/XC/consent_texts') {
      const t = over.texts ?? { status: 200, body: [MARKETING, TERMS] };
      r = typeof t === 'function' ? await t() : t;
    } else return new Response('{}', { status: 404 });
    if (r === 'network') throw new TypeError('Failed to fetch');
    return new Response(JSON.stringify(r.body), { status: r.status, headers: { 'content-type': 'application/json' } });
  });
  const textReads = () => urls.filter((u) => u.pathname === '/api/public/orgs/XC/consent_texts').length;
  return { fetchFn: fetchFn as unknown as typeof fetch, urls, textReads };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let states: TermsState[];
function mount(fetchFn: typeof fetch): void {
  states = [];
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <TermsText fetch={fetchFn} onState={(s) => states.push(s)} />, host);
}
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
});

const q = (id: string) => host.querySelector<HTMLElement>(`[data-testid="${id}"]`);
const last = () => states[states.length - 1];

describe('SF-5: TermsText reports its state', () => {
  it('loading: before the texts answer, it shows the loading line, reports loading, and no static terms line', async () => {
    setLang('en');
    let release!: (r: Reply) => void;
    const s = server({ texts: () => new Promise<Reply>((res) => (release = res)) });
    mount(s.fetchFn);
    await flush();
    expect(states).toEqual(['loading']);
    expect(q('terms-loading')?.textContent).toBe(table.en.termsLoading);
    expect(q('terms-body')).toBeNull();
    expect(host.textContent).not.toContain(table.en.terms);

    release({ status: 200, body: [TERMS] });
    await flush();
    expect(states).toEqual(['loading', 'ready']);
    expect(q('terms-loading')).toBeNull();
  });

  it('ready: shows the TERMS body and version, reports loading then ready, and no missing or failed line', async () => {
    setLang('en');
    mount(server().fetchFn);
    await flush();
    expect(states).toEqual(['loading', 'ready']);
    expect(q('terms-body')?.textContent).toContain('Terms body, version two.');
    expect(q('terms-version')?.textContent).toBe(table.en.termsVersion.replace('{version}', '2'));
    expect(host.textContent).not.toContain('We may send you offers.');
    expect(q('terms-missing')).toBeNull();
    expect(q('terms-failed')).toBeNull();
    expect(q('terms-retry')).toBeNull();
  });

  it.each<[string, Reply]>([
    ['no text in this language ([])', { status: 200, body: [] }],
    ['only a MARKETING text', { status: 200, body: [MARKETING] }],
  ])('missing: %s ⇒ the worded missing line, state missing, no body and no static terms line', async (_why, texts) => {
    setLang('en');
    mount(server({ texts }).fetchFn);
    await flush();
    expect(last()).toBe('missing');
    expect(states[0]).toBe('loading');
    expect(q('terms-missing')?.textContent).toBe(table.en.consentTextMissing);
    expect(q('terms-body')).toBeNull();
    expect(q('terms-failed')).toBeNull();
    expect(host.textContent).not.toContain(table.en.terms);
  });

  it.each<[string, { host?: Reply; texts?: Reply }]>([
    ['the host read fails (500)', { host: { status: 500, body: {} } }],
    ['the host answer has no org', { host: { status: 200, body: {} } }],
    ['the texts read fails (503)', { texts: { status: 503, body: {} } }],
    ['the network is down', { texts: 'network' }],
    ['the texts answer is not an array', { texts: { status: 200, body: { purpose: 'TERMS', version: 2, body: 'x' } } }],
  ])('failed: %s ⇒ the worded failed line and a retry button, state failed, no static terms line', async (_why, over) => {
    setLang('en');
    mount(server(over).fetchFn);
    await flush();
    expect(last()).toBe('failed');
    expect(states[0]).toBe('loading');
    expect(q('terms-failed')?.textContent ?? '').toContain(table.en.termsFailed);
    const retry = q('terms-retry') as HTMLButtonElement | null;
    expect(retry?.tagName).toBe('BUTTON');
    expect(retry?.getAttribute('type')).toBe('button');
    expect(retry?.textContent).toBe(table.en.termsRetry);
    expect(q('terms-body')).toBeNull();
    expect(q('terms-missing')).toBeNull();
    expect(host.textContent).not.toContain(table.en.terms);
  });

  it('the failed line is worded in the page language (th)', async () => {
    setLang('th');
    mount(server({ texts: { status: 503, body: {} } }).fetchFn);
    await flush();
    expect(q('terms-failed')?.textContent ?? '').toContain(table.th.termsFailed);
    expect(q('terms-retry')?.textContent).toBe(table.th.termsRetry);
  });

  it('retry after failed: it reads again, reports loading, and a good answer shows the text and reports ready', async () => {
    setLang('en');
    let answer: Reply = { status: 503, body: {} };
    const s = server({ texts: () => answer });
    mount(s.fetchFn);
    await flush();
    expect(last()).toBe('failed');
    expect(s.textReads()).toBe(1);

    answer = { status: 200, body: [TERMS] };
    expect(q('terms-retry'), 'retry button').not.toBeNull();
    q('terms-retry')!.click();
    await flush();
    expect(s.textReads()).toBe(2);
    expect(states).toEqual(['loading', 'failed', 'loading', 'ready']);
    expect(q('terms-body')?.textContent).toContain('Terms body, version two.');
    expect(q('terms-failed')).toBeNull();
    expect(q('terms-retry')).toBeNull();
  });

  it('retry that fails again stays failed, with the retry button still there', async () => {
    setLang('en');
    const s = server({ texts: 'network' });
    mount(s.fetchFn);
    await flush();
    expect(q('terms-retry'), 'retry button').not.toBeNull();
    q('terms-retry')!.click();
    await flush();
    expect(s.textReads()).toBe(2);
    expect(states).toEqual(['loading', 'failed', 'loading', 'failed']);
    expect(q('terms-retry')).not.toBeNull();
  });

  it('guard: without onState it still renders the text (existing props keep working)', async () => {
    setLang('en');
    host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <TermsText fetch={server().fetchFn} />, host);
    await flush();
    expect(q('terms-body')?.textContent).toContain('Terms body, version two.');
  });
});
