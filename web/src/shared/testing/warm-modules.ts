import { vi } from 'vitest';

export async function warmModules(...loaders: Array<() => Promise<unknown>>): Promise<void> {
  vi.stubGlobal(
    'fetch',
    vi.fn(
      async () =>
        new Response(JSON.stringify({ error: { code: 'unauthenticated' } }), {
          status: 401,
          headers: { 'content-type': 'application/json' },
        }),
    ),
  );
  try {
    for (const load of loaders) {
      await load();
    }
  } finally {
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
  }
}
