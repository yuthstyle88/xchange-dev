// #539 rework (FAIL at ccb062a0, PR 1303 issuecomment-6106453037; lane controller's ruling #539
// issuecomment-6106545095): the desk page mounted exactly as `main.tsx` wires it. `main.tsx` itself is imported, as the
// page's entry runs it, so its store, steps, summary and entry are the ones under test. With the #613 stub store
// (`createFrameStore`) in `main.tsx` the rate step (index 0, the step the page opens on) throws
// `store.config is not a function` and these tests go red.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../shared/i18n';
import { warmModules } from '../../shared/testing/warm-modules';

const HEADING_STEP_0 = 'เลือกเรทและจำนวนเงิน';
const SUMMARY = 'สรุปรายการ';

const ME = {
  kind: 'user',
  id: 'u-1',
  role: '',
  branch: 'MAIN',
  name: 'Ploy',
  username: 'ploy',
  branchName: 'XC-B',
  branches: [],
};

const CONFIG = {
  branchId: 'MAIN',
  branchName: 'XC-B',
  shiftOpen: true,
  adminUrl: '/?branch=MAIN',
  rates: [{ code: 'USD', unitSize: '1', buy: '32.10000000', sell: '32.90000000', publication: 'pub-1' }],
  paymentMethods: [],
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** The server the page meets on load: the session, the desk's config view, nothing else. */
function fakeServer() {
  const calls: string[] = [];
  const fn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    calls.push(`${String(init?.method ?? 'GET').toUpperCase()} ${url.split('?')[0]}`);
    const path = url.split('?')[0];
    if (path === '/api/v3/views/me') return json(200, { version: 1, changed: true, data: ME });
    if (path === '/api/v3/views/desk_config' || path === '/api/v3/views/staff_desk_config') {
      return json(200, { version: 1, changed: true, data: CONFIG });
    }
    return json(404, { error: { code: 'notFound', message: 'nf' } });
  });
  vi.stubGlobal('fetch', fn);
  return { calls };
}

let app: HTMLDivElement;

beforeAll(() => warmModules(() => import('./main')), 60_000);

beforeEach(() => {
  setLang('th');
  vi.resetModules();
  app = document.createElement('div');
  app.id = 'app';
  document.body.appendChild(app);
});

afterEach(() => {
  app.remove();
  window.history.replaceState({}, '', '/');
});

/** Runs the page's entry, as /desk loads it. A throw on first render rejects the import. */
async function bootAt(pathname: string) {
  window.history.replaceState({}, '', pathname);
  const server = fakeServer();
  await import('./main');
  return server;
}

describe('DeskPage as main.tsx wires it', () => {
  it('D1 /desk renders step 0 (rate) on the desk store without throwing, with the summary aside', async () => {
    const server = await bootAt('/desk');
    await vi.waitFor(() => expect(app.textContent).toContain(HEADING_STEP_0));
    expect(app.querySelector('.desk-aside')?.textContent).toContain(SUMMARY);
    expect(app.querySelector('ol.steps li[aria-current="step"]')).not.toBeNull();
    expect(server.calls).toContain('GET /api/v3/views/desk_config');
  });

  it("D2 the steps get the desk store: the rate step lists the config view's currencies", async () => {
    await bootAt('/desk');
    await vi.waitFor(() => {
      const options = [...app.querySelectorAll<HTMLOptionElement>('select[name="currency"] option')].map((o) => o.value);
      expect(options).toContain('USD');
    });
  });

  it("D3 /staff/desk reads the staff path's config view", async () => {
    const server = await bootAt('/staff/desk');
    await vi.waitFor(() => expect(app.textContent).toContain(HEADING_STEP_0));
    expect(server.calls).toContain('GET /api/v3/views/staff_desk_config');
    expect(server.calls).not.toContain('GET /api/v3/views/desk_config');
  });
});
