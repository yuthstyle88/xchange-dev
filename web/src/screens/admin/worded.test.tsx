import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { en } from '../../shared/i18n/en';
import { th } from '../../shared/i18n/th';
import { AdminShell } from './AdminShell';

let container: HTMLDivElement | null = null;
let dispose: (() => void) | null = null;

afterEach(() => {
  if (dispose) {
    dispose();
    dispose = null;
  }
  if (container) {
    container.remove();
    container = null;
  }
  setLang('th');
});

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function makeApi(branchErrorToThrow: unknown): ApiClient {
  return {
    async view<T>(name: string, _q: ViewParams = {}): Promise<ViewAnswer<T>> {
      if (name === 'me') {
        return {
          version: 1,
          changed: true,
          data: {
            id: 'e-1',
            branch: 'MAIN',
            permissions: ['CompatRead'],
            roles: [],
          } as unknown as T,
        };
      }
      if (name === 'admin_branches') {
        throw branchErrorToThrow;
      }
      throw new Error(`unexpected view ${name}`);
    },
    async page<R>(_name: string): Promise<R[]> {
      return [];
    },
    async act<B, R>(_area: string, _action: string, _body: B): Promise<R> {
      throw new Error('unexpected act');
    },
    async post<B, R>(_path: string, _body: B): Promise<R> {
      throw new Error('unexpected post');
    },
  } as unknown as ApiClient;
}

async function mount(api: ApiClient): Promise<HTMLDivElement> {
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(() => <AdminShell api={api} menus={[]} />, container);
  await settle();
  return container;
}

describe('WE-2: screens/admin/AdminShell failed call handling (docs/design/web-worded-error.md §4.2)', () => {
  it('(1) a network TypeError shows error.network in TH and EN', async () => {
    const err = new TypeError('Failed to fetch');
    const root = await mount(makeApi(err));
    const notice = root.querySelector('.notice-bar');
    expect(notice?.textContent).toContain(th['error.network']);

    // after setLang('en') shows en['error.network']
    setLang('en');
    await settle();
    expect(notice?.textContent).toContain(en['error.network']);
  });

  it('(2) a 500 refusal shows th[\'error.serverError\']', async () => {
    const err = new ApiError({ status: 500, code: 'dbConnectionFailed', message: 'fatal connection timeout' });
    const root = await mount(makeApi(err));
    const notice = root.querySelector('.notice-bar');
    expect(notice?.textContent).toContain(th['error.serverError']);
  });

  it('(3) an unknown code with a server message shows that message', async () => {
    const customMessage = 'สาขานี้ยังไม่พร้อมเปิดให้บริการ';
    const err = new ApiError({ status: 400, code: 'branchNotReadyForOperations', message: customMessage });
    const root = await mount(makeApi(err));
    const notice = root.querySelector('.notice-bar');
    expect(notice?.textContent).toContain(customMessage);
  });
});
