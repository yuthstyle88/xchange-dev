import { goChangePassword, needsPasswordChange } from '../session/password-change';

export interface ViewAnswer<T> {
  version: number;
  changed: boolean;
  data?: T;
  versions?: Record<string, number>;
}

export interface ViewParams<F = unknown> {
  branch?: string;
  since?: number;
  limit?: number;
  filter?: F;
}

export interface PageParams<F = unknown> {
  branch?: string;
  before?: string;
  limit?: number;
  filter?: F;
  /** EX-9 #1147: marks the read as an export; the server gates it on `ReportsExport` and audits it. */
  purpose?: 'export';
}

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

const defaultFetch: Fetch = async (input, init) => {
  const res = await fetch(input, init);
  if (res.status === 403) {
    let code: string | undefined;
    try {
      const data = await res.clone().json();
      code = data?.error?.code ?? data?.code;
    } catch {
      // not json
    }
    if (needsPasswordChange(res.status, code)) {
      goChangePassword();
    }
  }
  return res;
};

export class ApiError extends Error {
  readonly code: string;
  readonly field?: string;
  readonly reason?: string;
  readonly status: number;

  constructor(init: { code: string; field?: string; reason?: string; message: string; status: number }) {
    super(init.message);
    this.name = 'ApiError';
    this.code = init.code;
    this.field = init.field;
    this.reason = init.reason;
    this.status = init.status;
  }
}

export class ApiClient {
  private readonly fetchFn: Fetch;

  constructor(fetchFn?: Fetch) {
    this.fetchFn = fetchFn ?? defaultFetch;
  }

  private async parseError(res: Response): Promise<never> {
    const text = await res.text();
    try {
      const json = JSON.parse(text);
      if (json && typeof json === 'object' && json.error) {
        if (typeof json.error === 'string') {
          throw new ApiError({
            code: 'refused',
            message: json.error,
            status: res.status,
          });
        }
        const reason = typeof json.error.reason === 'string' ? json.error.reason : undefined;
        throw new ApiError({
          code: json.error.code ?? 'unknown',
          field: json.error.field,
          reason,
          message: json.error.message ?? text,
          status: res.status,
        });
      }
    } catch (e) {
      if (e instanceof ApiError) throw e;
    }
    throw new ApiError({
      code: 'httpError',
      message: text || res.statusText,
      status: res.status,
    });
  }

  async view<T, F = unknown>(name: string, q: ViewParams<F> = {}): Promise<ViewAnswer<T>> {
    const params = new URLSearchParams();
    if (q.branch !== undefined) params.set('branch', q.branch);
    if (q.since !== undefined) params.set('since', String(q.since));
    if (q.limit !== undefined) params.set('limit', String(q.limit));
    if (q.filter && typeof q.filter === 'object') {
      for (const [k, v] of Object.entries(q.filter)) {
        if (v === undefined || v === null || v === '') continue;
        params.set(k, String(v));
      }
    }

    const qs = params.toString();
    const url = `/api/v3/views/${name}${qs ? `?${qs}` : ''}`;
    const res = await this.fetchFn(url, {
      method: 'GET',
      credentials: 'same-origin',
    });

    if (!res.ok) {
      await this.parseError(res);
    }

    const json = (await res.json()) as {
      version: number;
      changed: boolean;
      data?: T;
      versions?: unknown;
    };
    const answer: ViewAnswer<T> = {
      version: json.version,
      changed: json.changed,
    };
    if ('data' in json && json.data !== undefined) {
      answer.data = json.data;
    }
    if (typeof json.versions === 'object' && json.versions !== null && !Array.isArray(json.versions)) {
      answer.versions = json.versions as Record<string, number>;
    }
    return answer;
  }

  async viewBare<T>(name: string, q: { branch?: string } = {}): Promise<T> {
    const params = new URLSearchParams();
    if (q.branch !== undefined) params.set('branch', q.branch);

    const qs = params.toString();
    const url = `/api/v3/views/${name}${qs ? `?${qs}` : ''}`;
    const res = await this.fetchFn(url, {
      method: 'GET',
      credentials: 'same-origin',
    });

    if (!res.ok) {
      await this.parseError(res);
    }

    return (await res.json()) as T;
  }

  async page<R, F = unknown>(name: string, q: PageParams<F> = {}): Promise<R[]> {
    const params = new URLSearchParams();
    if (q.branch !== undefined) params.set('branch', q.branch);
    if (q.before !== undefined) params.set('before', q.before);
    if (q.limit !== undefined) params.set('limit', String(q.limit));
    if (q.purpose !== undefined) params.set('purpose', q.purpose);
    if (q.filter && typeof q.filter === 'object') {
      for (const [k, v] of Object.entries(q.filter)) {
        if (v === undefined || v === null || v === '') continue;
        params.set(k, String(v));
      }
    }

    const qs = params.toString();
    const url = `/api/v3/views/${name}${qs ? `?${qs}` : ''}`;
    const res = await this.fetchFn(url, {
      method: 'GET',
      credentials: 'same-origin',
    });

    if (!res.ok) {
      await this.parseError(res);
    }

    const json = await res.json();
    if (Array.isArray(json)) return json as R[];
    if (json && typeof json === 'object' && Array.isArray(json.data)) return json.data as R[];
    return [];
  }

  async act<B, R>(area: string, action: string, body: B): Promise<R> {
    const url = `/api/v3/${area}/${action}`;
    const res = await this.fetchFn(url, {
      method: 'POST',
      credentials: 'same-origin',
      headers: {
        'x-xchange-request': '1',
        'content-type': 'application/json',
      },
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      await this.parseError(res);
    }

    return (await res.json()) as R;
  }

  post<B, R>(path: string, body: B): Promise<R> {
    if (!path.startsWith('/api/') || path.startsWith('/api/v3/')) {
      throw new Error(`Invalid path for post: ${path}`);
    }
    return (async () => {
      const res = await this.fetchFn(path, {
        method: 'POST',
        credentials: 'same-origin',
        headers: {
          'x-xchange-request': '1',
          'content-type': 'application/json',
        },
        body: JSON.stringify(body),
      });

      if (!res.ok) {
        await this.parseError(res);
      }

      return (await res.json()) as R;
    })();
  }

  command<B, R>(method: string, path: string, body: B, key: string): Promise<R> {
    if (!key || key.trim() === '') {
      throw new Error('Idempotency key must not be empty');
    }
    const url = path.startsWith('/') ? path : `/${path}`;
    return (async () => {
      const res = await this.fetchFn(url, {
        method,
        credentials: 'same-origin',
        headers: {
          'x-xchange-request': '1',
          'idempotency-key': key,
          'content-type': 'application/json',
        },
        body: JSON.stringify(body),
      });

      if (!res.ok) {
        await this.parseError(res);
      }

      return (await res.json()) as R;
    })();
  }

  /** A raw upload (#539): the Blob is the body and `contentType` its type; sent and refused like `command`. */
  postBytes<R>(path: string, body: Blob, contentType: string, idempotencyKey: string): Promise<R> {
    if (!idempotencyKey || idempotencyKey.trim() === '') {
      throw new Error('Idempotency key must not be empty');
    }
    return (async () => {
      const res = await this.fetchFn(path, {
        method: 'POST',
        credentials: 'same-origin',
        headers: {
          'x-xchange-request': '1',
          'idempotency-key': idempotencyKey,
          'content-type': contentType,
        },
        body,
      });

      if (!res.ok) {
        await this.parseError(res);
      }

      return (await res.json()) as R;
    })();
  }

  /** A multipart/form-data upload (#2038): `form` is the body and fetch sets the boundary; sent and refused like `command`. */
  postForm<R>(path: string, form: FormData, idempotencyKey: string): Promise<R> {
    if (!idempotencyKey || idempotencyKey.trim() === '') {
      throw new Error('Idempotency key must not be empty');
    }
    return (async () => {
      const res = await this.fetchFn(path, {
        method: 'POST',
        credentials: 'same-origin',
        headers: {
          'x-xchange-request': '1',
          'idempotency-key': idempotencyKey,
        },
        body: form,
      });

      if (!res.ok) {
        await this.parseError(res);
      }

      return (await res.json()) as R;
    })();
  }
}

