import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient } from '../api/client';
import { setLang } from '../i18n';
import { th } from '../i18n/th';
import { en } from '../i18n/en';
import { createNotifications } from './notifications';

let handler: ((path: string, method: string) => Promise<Response> | Response) | null = null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function defaultFetch(path: string, method: string): Response {
  if (path === '/api/v3/views/my_notifications' && method === 'GET') {
    return json(200, { version: 1, changed: true, data: { items: [] } });
  }
  return json(200, { ok: true });
}

beforeEach(() => {
  setLang('th');
  handler = null;
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(url, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (handler) {
      const res = await handler(u.pathname, method);
      if (res) return res;
    }
    return defaultFetch(u.pathname, method);
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  setLang('th');
  handler = null;
});

async function flushPromises() {
  for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 10));
}

const UNKEYED_MESSAGE = 'ข้อผิดพลาดการแจ้งเตือนที่ไม่รู้จักจากเซิร์ฟเวอร์';

describe('WE-45: shared/notifications/ worded errors (web-worded-error.md §4.2)', () => {
  describe('catch 1: load (sets loadError)', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      handler = (path) => {
        if (path === '/api/v3/views/my_notifications') throw new TypeError('Failed to fetch');
        return defaultFetch(path, 'GET');
      };

      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        expect(model.loadError()).toBe(th['error.network']);
        dispose();
      });

      setLang('en');
      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        expect(model.loadError()).toBe(en['error.network']);
        dispose();
      });
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      handler = (path) => {
        if (path === '/api/v3/views/my_notifications') {
          return json(500, { error: { code: 'custom500Code', message: 'fatal db timeout' } });
        }
        return defaultFetch(path, 'GET');
      };

      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        expect(model.loadError()).toBe(th['error.serverError']);
        dispose();
      });
    });

    it('(3) unkeyed code with server message shows message', async () => {
      handler = (path) => {
        if (path === '/api/v3/views/my_notifications') {
          return json(400, { error: { code: 'unkeyedLoadCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(path, 'GET');
      };

      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        expect(model.loadError()).toBe(UNKEYED_MESSAGE);
        dispose();
      });
    });
  });

  describe('catch 2: markOneRead (sets actionError)', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      handler = (path, method) => {
        if (path.includes('/read') && method === 'POST') throw new TypeError('Failed to fetch');
        return defaultFetch(path, method);
      };

      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        await model.markOneRead('n-1');
        expect(model.actionError()).toBe(th['error.network']);
        dispose();
      });

      setLang('en');
      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        await model.markOneRead('n-1');
        expect(model.actionError()).toBe(en['error.network']);
        dispose();
      });
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      handler = (path, method) => {
        if (path.includes('/read') && method === 'POST') {
          return json(500, { error: { code: 'custom500Code', message: 'fatal db timeout' } });
        }
        return defaultFetch(path, method);
      };

      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        await model.markOneRead('n-1');
        expect(model.actionError()).toBe(th['error.serverError']);
        dispose();
      });
    });

    it('(3) unkeyed code with server message shows message', async () => {
      handler = (path, method) => {
        if (path.includes('/read') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedMarkOneCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(path, method);
      };

      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        await model.markOneRead('n-1');
        expect(model.actionError()).toBe(UNKEYED_MESSAGE);
        dispose();
      });
    });
  });

  describe('catch 3: markAllRead (sets actionError)', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      handler = (path, method) => {
        if (path === '/api/app/notifications/read-all' && method === 'POST') {
          throw new TypeError('Failed to fetch');
        }
        return defaultFetch(path, method);
      };

      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        await model.markAllRead();
        expect(model.actionError()).toBe(th['error.network']);
        dispose();
      });

      setLang('en');
      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        await model.markAllRead();
        expect(model.actionError()).toBe(en['error.network']);
        dispose();
      });
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      handler = (path, method) => {
        if (path === '/api/app/notifications/read-all' && method === 'POST') {
          return json(500, { error: { code: 'custom500Code', message: 'fatal db timeout' } });
        }
        return defaultFetch(path, method);
      };

      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        await model.markAllRead();
        expect(model.actionError()).toBe(th['error.serverError']);
        dispose();
      });
    });

    it('(3) unkeyed code with server message shows message', async () => {
      handler = (path, method) => {
        if (path === '/api/app/notifications/read-all' && method === 'POST') {
          return json(400, { error: { code: 'unkeyedMarkAllCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(path, method);
      };

      await createRoot(async (dispose) => {
        const model = createNotifications(new ApiClient());
        await flushPromises();
        await model.markAllRead();
        expect(model.actionError()).toBe(UNKEYED_MESSAGE);
        dispose();
      });
    });
  });
});
