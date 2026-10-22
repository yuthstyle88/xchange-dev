// C14 up-front tests (feat/fe-customer-2 skeleton): TermsText shows the published TERMS text in the page language
// from the public `GET /api/public/orgs/{org}/consent_texts?lang=` (BE contract #3409). The implementer makes these pass
// and must not edit them.
// SF-5 (docs/design/web-silent-failures.md §6, owner 2026-10-22 "A") retired the five "falls back to the static terms
// line when …" rows: a missing or failed text no longer shows the static line. TermsText.state.test.tsx pins what it
// shows instead.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { TermsText } from './TermsText';

type Reply = { status: number; body: unknown } | 'network';

const TEXTS = [
  { purpose: 'MARKETING', version: 1, body: 'We may send you offers.' },
  { purpose: 'TERMS', version: 2, body: 'Terms body, version two.' },
];

function server(over: { host?: Reply; texts?: Reply } = {}) {
  const urls: URL[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL) => {
    const u = new URL(String(input), 'http://localhost');
    urls.push(u);
    let r: Reply;
    if (u.pathname === '/api/public/host') r = over.host ?? { status: 200, body: { org: 'XC' } };
    else if (u.pathname === '/api/public/orgs/XC/consent_texts') r = over.texts ?? { status: 200, body: TEXTS };
    else return new Response('{}', { status: 404 });
    if (r === 'network') throw new TypeError('Failed to fetch');
    return new Response(JSON.stringify(r.body), { status: r.status, headers: { 'content-type': 'application/json' } });
  });
  return { fetchFn: fetchFn as unknown as typeof fetch, urls };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
function mount(fetchFn: typeof fetch): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <TermsText fetch={fetchFn} />, host);
}
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('TermsText (C14)', () => {
  it('reads the host org, then that org\'s texts in the page language, and shows the TERMS body and version', async () => {
    setLang('th');
    const s = server();
    mount(s.fetchFn);
    await flush();
    const texts = s.urls.find((u) => u.pathname === '/api/public/orgs/XC/consent_texts');
    expect(texts?.searchParams.get('lang')).toBe('th');
    expect(host.querySelector('[data-testid="terms-body"]')?.textContent).toContain('Terms body, version two.');
    expect(host.querySelector('[data-testid="terms-version"]')?.textContent).toContain('2');
    expect(host.textContent).not.toContain('We may send you offers.');
  });

  it('asks in the current language', async () => {
    setLang('zh');
    const s = server();
    mount(s.fetchFn);
    await flush();
    expect(s.urls.find((u) => u.pathname.endsWith('/consent_texts'))?.searchParams.get('lang')).toBe('zh');
  });
});
