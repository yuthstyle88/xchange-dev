// V2-parts (#476): the CSV helpers (ticket §7 `csv.ts`, §9, design §12.2, §12.8, behaviour P3).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// - `csvCell(v)` is v2's guard (`admin-html:84`): `String(v ?? '')`; a cell matching
//   `^[\s]*[=+@\-]` gets `'` in front (leading white space counts, unlike the devices CSV); every
//   `"` doubled; the whole wrapped in `"`.
// - `walkPages(api, view, {branch, filter})` reads the paged view through `ApiClient.page` with
//   `limit: 100`, no `before` on the first read and then `before` = the last row's `id`, the same
//   branch and filter every time, until a page shorter than 100 (§12.8: no export endpoint; the
//   CSV is built from the server's rows, so the figures stay the server's, W7).
// - `downloadCsv(name, headers, rows)` (`admin-html:170`): a BOM, the header line and one line per
//   row, each cell through `csvCell`, cells joined by `,`, lines by `\r\n` with no break at the
//   end; a `text/csv;charset=utf-8` Blob behind an object URL, a link with `download = name`
//   clicked, the URL revoked after 1 s.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ApiClient, PageParams } from '../../../shared/api/client';
import { csvCell, downloadCsv, walkPages } from './csv';

describe('csvCell (#476 P3, §9)', () => {
  it('wraps a plain cell in quotes', () => {
    expect(csvCell('abc')).toBe('"abc"');
    expect(csvCell('a=b')).toBe('"a=b"');
    expect(csvCell('x-1')).toBe('"x-1"');
  });

  it("puts ' before a cell starting with = + @ or -, after any leading white space (§9)", () => {
    expect(csvCell('=SUM(A1)')).toBe(`"'=SUM(A1)"`);
    expect(csvCell('  +1')).toBe(`"'  +1"`);
    expect(csvCell('@cmd')).toBe(`"'@cmd"`);
    expect(csvCell('-5')).toBe(`"'-5"`);
    expect(csvCell('\t=x')).toBe(`"'\t=x"`);
    expect(csvCell('\n-1')).toBe(`"'\n-1"`);
  });

  it('doubles every quote mark, also in a guarded cell', () => {
    expect(csvCell('a"b"c')).toBe('"a""b""c"');
    expect(csvCell('=HYPERLINK("http://x")')).toBe(`"'=HYPERLINK(""http://x"")"`);
  });

  it('null and undefined are empty; a number is its text', () => {
    expect(csvCell(null)).toBe('""');
    expect(csvCell(undefined)).toBe('""');
    expect(csvCell(0)).toBe('"0"');
    expect(csvCell('')).toBe('""');
  });
});

// ── walkPages ────────────────────────────────────────────────────────────────

interface Row {
  id: string;
  amount: string;
}

interface Filter {
  from: string;
  to: string;
  status: string;
}

interface Read {
  name: string;
  q: PageParams<Filter>;
}

function rows(prefix: string, n: number): Row[] {
  return Array.from({ length: n }, (_, i) => ({ id: `${prefix}${i}`, amount: `${i}.10` }));
}

/** A fake ApiClient whose `page` answers the scripted pages in order and records each read. */
function fakeApi(pages: Row[][]): { api: ApiClient; reads: Read[] } {
  const reads: Read[] = [];
  let next = 0;
  const api = {
    page: async (name: string, q: PageParams<Filter> = {}) => {
      reads.push({ name, q: { ...q } });
      const page = pages[next++];
      if (page === undefined) throw new Error(`no page ${next} scripted`);
      return page;
    },
    view: async () => {
      throw new Error('walkPages reads pages only');
    },
    post: async () => {
      throw new Error('walkPages reads pages only');
    },
  };
  return { api: api as unknown as ApiClient, reads };
}

const FILTER: Filter = { from: '2026-10-01', to: '2026-10-08', status: 'COMPLETED' };

describe('walkPages (#476 P3, §9, §12.8)', () => {
  it('reads 100 at a time with before = the last id, until a short page; all rows in order', async () => {
    const p1 = rows('a', 100);
    const p2 = rows('b', 100);
    const p3 = rows('c', 37);
    const { api, reads } = fakeApi([p1, p2, p3]);
    const all = await walkPages<Row, Filter>(api, 'admin_transactions', { branch: 'B1', filter: FILTER });
    expect(all).toEqual([...p1, ...p2, ...p3]);
    expect(reads.map((r) => r.name)).toEqual(['admin_transactions', 'admin_transactions', 'admin_transactions']);
    expect(reads.map((r) => r.q.limit)).toEqual([100, 100, 100]);
    expect(reads.map((r) => r.q.before)).toEqual([undefined, 'a99', 'b99']);
    expect(reads.map((r) => r.q.branch)).toEqual(['B1', 'B1', 'B1']);
    for (const r of reads) {
      expect(r.q.filter).toEqual(FILTER);
    }
  });

  it('exactly 100 rows: a second read answers [], the walk stops with 100 rows (§9)', async () => {
    const p1 = rows('a', 100);
    const { api, reads } = fakeApi([p1, []]);
    const all = await walkPages<Row, Filter>(api, 'admin_transactions', { branch: 'B1', filter: FILTER });
    expect(all.length).toBe(100);
    expect(all).toEqual(p1);
    expect(reads.length).toBe(2);
    expect(reads[1]!.q.before).toBe('a99');
  });

  it('an empty first page: one read, no rows', async () => {
    const { api, reads } = fakeApi([[]]);
    expect(await walkPages<Row, Filter>(api, 'admin_slips', { branch: 'MAIN', filter: FILTER })).toEqual([]);
    expect(reads.length).toBe(1);
    expect(reads[0]!.q.before).toBeUndefined();
  });

  it('a short first page: one read', async () => {
    const { api, reads } = fakeApi([rows('a', 99)]);
    expect((await walkPages<Row, Filter>(api, 'admin_transactions', { branch: 'B1', filter: FILTER })).length).toBe(99);
    expect(reads.length).toBe(1);
  });

  it('a failed read fails the walk', async () => {
    const { api } = fakeApi([rows('a', 100)]);
    await expect(walkPages<Row, Filter>(api, 'admin_transactions', { branch: 'B1', filter: FILTER })).rejects.toThrow();
  });
});

// ── downloadCsv ──────────────────────────────────────────────────────────────

describe('downloadCsv (#476 P3, admin-html:170)', () => {
  let blobs: Blob[];
  let revoked: string[];
  let clicks: Array<{ href: string; download: string; inDocument: boolean }>;
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;

  beforeEach(() => {
    vi.useFakeTimers();
    blobs = [];
    revoked = [];
    clicks = [];
    URL.createObjectURL = ((blob: Blob) => {
      blobs.push(blob);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = ((url: string) => {
      revoked.push(url);
    }) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      clicks.push({ href: this.getAttribute('href') ?? '', download: this.download, inDocument: this.isConnected });
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
  });

  async function bytesOf(blob: Blob): Promise<Uint8Array> {
    return new Uint8Array(await blob.arrayBuffer());
  }

  it('a BOM, the header, one CRLF line per row through csvCell, no break at the end', async () => {
    downloadCsv(
      'XCHANGE-LIVE-Main-2026-10-08.csv',
      ['Document Ref', 'Amount', 'Note'],
      [
        ['TX-1', '100.00', '=SUM(A1)'],
        ['TX-2', '90071992547409.93', 'say "hi"'],
      ],
    );
    expect(blobs.length).toBe(1);
    const bytes = await bytesOf(blobs[0]!);
    expect([...bytes.slice(0, 3)]).toEqual([0xef, 0xbb, 0xbf]);
    const text = new TextDecoder('utf-8', { ignoreBOM: true }).decode(bytes);
    expect(text).toBe(
      '﻿' +
        '"Document Ref","Amount","Note"' +
        '\r\n' +
        `"TX-1","100.00","'=SUM(A1)"` +
        '\r\n' +
        '"TX-2","90071992547409.93","say ""hi"""',
    );
  });

  it('a text/csv;charset=utf-8 Blob, downloaded under the name given', () => {
    downloadCsv('XCHANGE-LIVE-Main-2026-10-08.csv', ['a'], [['1']]);
    expect(blobs[0]!.type).toBe('text/csv;charset=utf-8');
    expect(clicks.length).toBe(1);
    expect(clicks[0]!.download).toBe('XCHANGE-LIVE-Main-2026-10-08.csv');
    expect(clicks[0]!.href).toBe('blob:test/1');
  });

  it('the object URL is revoked 1 s after the click, not before', () => {
    downloadCsv('x.csv', ['a'], []);
    expect(revoked).toEqual([]);
    vi.advanceTimersByTime(999);
    expect(revoked).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(revoked).toEqual(['blob:test/1']);
  });

  it('no rows: the BOM and the header only', async () => {
    downloadCsv('x.csv', ['Branch ID', 'Branch'], []);
    const text = new TextDecoder('utf-8', { ignoreBOM: true }).decode(await bytesOf(blobs[0]!));
    expect(text).toBe('﻿"Branch ID","Branch"');
  });
});
