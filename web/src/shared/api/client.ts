export interface ViewAnswer<T> {
  version: number;
  changed: boolean;
  data?: T;
}

export interface ViewParams {
  branch?: string;
  since?: number;
  limit?: number;
}

export interface PageParams {
  branch?: string;
  before?: string;
  limit?: number;
}

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

export class ApiError extends Error {
  readonly code: string;
  readonly field?: string;
  readonly status: number;

  constructor(init: { code: string; field?: string; message: string; status: number }) {
    super(init.message);
    this.name = 'ApiError';
    this.code = init.code;
    this.field = init.field;
    this.status = init.status;
  }
}

export class ApiClient {
  private readonly fetchFn: Fetch;

  constructor(fetchFn?: Fetch) {
    this.fetchFn = fetchFn ?? ((input, init) => fetch(input, init));
  }

  private async parseError(res: Response): Promise<never> {
    const text = await res.text();
    try {
      const json = JSON.parse(text);
      if (json && typeof json === 'object' && json.error) {
        throw new ApiError({
          code: json.error.code ?? 'unknown',
          field: json.error.field,
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

  async view<T>(name: string, q: ViewParams = {}): Promise<ViewAnswer<T>> {
    const params = new URLSearchParams();
    if (q.branch !== undefined) params.set('branch', q.branch);
    if (q.since !== undefined) params.set('since', String(q.since));
    if (q.limit !== undefined) params.set('limit', String(q.limit));

    const qs = params.toString();
    const url = `/api/v3/views/${name}${qs ? `?${qs}` : ''}`;
    const res = await this.fetchFn(url, {
      method: 'GET',
      credentials: 'same-origin',
    });

    if (!res.ok) {
      await this.parseError(res);
    }

    const json = (await res.json()) as { version: number; changed: boolean; data?: T };
    const answer: ViewAnswer<T> = {
      version: json.version,
      changed: json.changed,
    };
    if ('data' in json && json.data !== undefined) {
      answer.data = json.data;
    }
    return answer;
  }

  async page<R>(name: string, q: PageParams = {}): Promise<R[]> {
    const params = new URLSearchParams();
    if (q.branch !== undefined) params.set('branch', q.branch);
    if (q.before !== undefined) params.set('before', q.before);
    if (q.limit !== undefined) params.set('limit', String(q.limit));

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
}
