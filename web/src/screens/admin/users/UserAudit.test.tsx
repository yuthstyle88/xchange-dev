import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from './i18n';
import { UserAudit } from './UserAudit';

interface MockAuditRow {
  id: string;
  at: string;
  actorId: string;
  actorName: string;
  subjectId: string;
  subjectName: string;
  action: string;
  before: unknown | null;
  after: unknown | null;
  ip: string | null;
}

const B01 = 'XC-B01';
const B02 = 'XC-B02';

const SAMPLE_ROW_1: MockAuditRow = {
  id: 'audit-1',
  at: '2026-10-06T02:30:00Z',
  actorId: 'admin-1',
  actorName: 'Somchai Admin',
  subjectId: 'user-1',
  subjectName: 'Malee Teller',
  action: 'GRANT',
  before: null,
  after: { role: 'TELLER', branch: B01 },
  ip: '203.0.113.10',
};

const SAMPLE_ROW_2: MockAuditRow = {
  id: 'audit-2',
  at: '2026-10-07T08:15:00Z',
  actorId: 'admin-2',
  actorName: 'Somsak Manager',
  subjectId: 'user-2',
  subjectName: 'Wichai Staff',
  action: 'SUSPEND',
  before: { status: 'ACTIVE' },
  after: { status: 'SUSPENDED' },
  ip: null,
};

let requests: string[] = [];
let rowsHandler: ((url: URL) => Promise<Response> | Response) | null = null;
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function refusal(status: number, code: string, message: string): Response {
  return json(status, { error: { code, message } });
}

beforeEach(() => {
  setLang('th');
  requests = [];
  rowsHandler = null;
  document.body.innerHTML = '';
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const u = new URL(String(input), 'http://localhost');
      requests.push(u.pathname + u.search);
      if (rowsHandler) {
        return rowsHandler(u);
      }
      if (u.pathname === '/api/v3/views/admin_user_audit') {
        return json(200, [SAMPLE_ROW_1, SAMPLE_ROW_2]);
      }
      return json(404, { error: { code: 'notFound', message: 'not found' } });
    }),
  );
});

afterEach(() => {
  while (disposers.length) disposers.pop()!();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

async function settle(): Promise<void> {
  for (let round = 0; round < 6; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function mount(initialShown = true, initialBranch = B01) {
  const [shown, setShown] = createSignal(initialShown);
  const [branch, setBranch] = createSignal(initialBranch);
  const c = document.createElement('div');
  document.body.appendChild(c);
  disposers.push(render(() => <UserAudit shown={shown} branch={branch} />, c));
  return { c, setShown, setBranch };
}

describe('UserAudit component (§8 and §9 specifications)', () => {
  it('row 1 (§8): shown() false makes no request to admin_user_audit', async () => {
    mount(false, B01);
    await settle();
    expect(requests.filter((r) => r.startsWith('/api/v3/views/admin_user_audit'))).toHaveLength(0);
  });

  it('row 2 (§8): shown() becomes true makes one GET with limit=100 and renders rows in server order', async () => {
    const { c } = mount(true, B01);
    await settle();
    const auditGets = requests.filter((r) => r.startsWith('/api/v3/views/admin_user_audit'));
    expect(auditGets).toHaveLength(1);
    expect(auditGets[0]).toBe(`/api/v3/views/admin_user_audit?branch=${B01}&limit=100`);

    const tableRows = c.querySelectorAll('tbody tr');
    expect(tableRows).toHaveLength(2);
    expect(tableRows[0]?.textContent).toContain('Somchai Admin');
    expect(tableRows[0]?.textContent).toContain('Malee Teller');
    expect(tableRows[0]?.textContent).toContain('GRANT');
    expect(tableRows[1]?.textContent).toContain('Somsak Manager');
    expect(tableRows[1]?.textContent).toContain('Wichai Staff');
    expect(tableRows[1]?.textContent).toContain('SUSPEND');
  });

  it('row 3 (§8): branch changes while shown triggers new GET and replaces rows', async () => {
    const rowBranch2: MockAuditRow = {
      ...SAMPLE_ROW_1,
      id: 'audit-b2',
      actorName: 'Branch 2 Admin',
    };
    rowsHandler = (u: URL) => {
      const branch = u.searchParams.get('branch');
      if (branch === B02) return json(200, [rowBranch2]);
      return json(200, [SAMPLE_ROW_1]);
    };

    const { c, setBranch } = mount(true, B01);
    await settle();
    expect(c.textContent).toContain('Somchai Admin');
    expect(c.textContent).not.toContain('Branch 2 Admin');

    setBranch(B02);
    await settle();

    const auditGets = requests.filter((r) => r.startsWith('/api/v3/views/admin_user_audit'));
    expect(auditGets).toHaveLength(2);
    expect(auditGets[1]).toBe(`/api/v3/views/admin_user_audit?branch=${B02}&limit=100`);
    expect(c.textContent).toContain('Branch 2 Admin');
    expect(c.textContent).not.toContain('Somchai Admin');
  });

  it('row 4 (§8): older branch answer arriving after newer is ignored (sequence counter)', async () => {
    let resolveFirst!: (res: Response) => void;
    const firstPromise = new Promise<Response>((resolve) => {
      resolveFirst = resolve;
    });

    const rowBranch2: MockAuditRow = {
      ...SAMPLE_ROW_1,
      id: 'audit-b2',
      actorName: 'Newer Branch Admin',
    };

    rowsHandler = (u: URL) => {
      const branch = u.searchParams.get('branch');
      if (branch === B01) return firstPromise;
      return json(200, [rowBranch2]);
    };

    const { c, setBranch } = mount(true, B01);
    await settle();

    // Now switch branch to B02 before B01 resolves
    setBranch(B02);
    await settle();
    expect(c.textContent).toContain('Newer Branch Admin');

    // Now let B01 resolve late
    resolveFirst(json(200, [SAMPLE_ROW_1]));
    await settle();

    // Newer branch B02 rows must stay, older B01 dropped
    expect(c.textContent).toContain('Newer Branch Admin');
    expect(c.textContent).not.toContain('Somchai Admin');
  });

  it('rows 5 & 6 (§8): before / after null shows em dash; object shows JSON text in <code>', async () => {
    const { c } = mount(true, B01);
    await settle();

    const firstRow = c.querySelectorAll('tbody tr')[0];
    expect(firstRow?.textContent).toContain('—');

    const codeEl = firstRow?.querySelector('code');
    expect(codeEl).not.toBeNull();
    expect(codeEl?.textContent).toBe(JSON.stringify({ role: 'TELLER', branch: B01 }));
  });

  it('row 7 (§8): ip null shows em dash; present ip is shown as given', async () => {
    const { c } = mount(true, B01);
    await settle();

    const rows = c.querySelectorAll('tbody tr');
    expect(rows[0]?.textContent).toContain('203.0.113.10');
    expect(rows[1]?.textContent).toContain('—');
  });

  it('row 8 (§8) & §9 edge: at shown in Asia/Bangkok (02:30 UTC -> 09:30 Bangkok)', async () => {
    const { c } = mount(true, B01);
    await settle();

    const firstRow = c.querySelectorAll('tbody tr')[0];
    // 2026-10-06T02:30:00Z in Asia/Bangkok (+07:00) is 09:30
    expect(firstRow?.textContent).toContain('09:30');
  });

  it('row 9 (§8): no rows shows auditEmpty and no table', async () => {
    rowsHandler = () => json(200, []);
    const { c } = mount(true, B01);
    await settle();

    expect(c.querySelector('table')).toBeNull();
    expect(c.textContent).toContain(th.auditEmpty);
  });

  it('row 10 (§8): read refusal shows worded error and no table', async () => {
    rowsHandler = () => refusal(403, 'forbidden', 'ไม่มีสิทธิ์เข้าถึงบันทึกการตรวจสอบ');
    const { c } = mount(true, B01);
    await settle();

    expect(c.querySelector('table')).toBeNull();
    const alert = c.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent).toContain('ไม่มีสิทธิ์เข้าถึงบันทึกการตรวจสอบ');
  });

  it('row 11 (§8): component is read-only with no inputs or buttons', async () => {
    const { c } = mount(true, B01);
    await settle();

    const interactive = c.querySelectorAll('input, button, select, textarea, [contenteditable="true"]');
    expect(interactive).toHaveLength(0);
  });

  it('§9 edge: before/after containing <script> is text-escaped with no script element in DOM', async () => {
    const xssRow: MockAuditRow = {
      ...SAMPLE_ROW_1,
      id: 'xss-row',
      before: { note: '<script>alert(1)</script>' },
      after: { tag: '<img src=x onerror=alert(1)>' },
    };
    rowsHandler = () => json(200, [xssRow]);

    const { c } = mount(true, B01);
    await settle();

    expect(c.querySelector('script')).toBeNull();
    expect(c.querySelector('img')).toBeNull();
    expect(c.textContent).toContain('<script>alert(1)</script>');
    expect(c.textContent).toContain('<img src=x onerror=alert(1)>');
  });

  it('§9 edge: shown() toggling true -> false -> true performs two reads', async () => {
    const { setShown } = mount(true, B01);
    await settle();
    expect(requests.filter((r) => r.startsWith('/api/v3/views/admin_user_audit'))).toHaveLength(1);

    setShown(false);
    await settle();
    expect(requests.filter((r) => r.startsWith('/api/v3/views/admin_user_audit'))).toHaveLength(1);

    setShown(true);
    await settle();
    expect(requests.filter((r) => r.startsWith('/api/v3/views/admin_user_audit'))).toHaveLength(2);
  });
});
