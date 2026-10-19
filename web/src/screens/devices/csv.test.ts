// V1-devices-screen (#404): the events CSV (ticket §7 `csv.ts`, §9, §10.1 D17). Written before the
// implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests
// in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb:src/devices-public/devices.js:14`): a BOM, then the header
// `เวลา,ผู้ทำงาน,เหตุการณ์,เลขอ้างอิง,รายละเอียด` and one line per row `at, actor, kind, ref,
// detail`, lines joined by `\r\n` with no line break at the end. Each cell is `String(x ?? '')`,
// a leading `=`, `+`, `@` or `-` prefixed with `'`, then every `"` doubled, then wrapped in `"`.
// The kind is the raw code, not its label; `at` is the view's string, not a formatted time.
// v3's `detail` is already the JSON text v2 built with `JSON.stringify`, so it goes in as it is.
import { beforeEach, describe, expect, it } from 'vitest';
import { makeDeviceEventRow } from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { eventsCsv } from './csv';

const BOM = '﻿';
const HEADER = '"เวลา","ผู้ทำงาน","เหตุการณ์","เลขอ้างอิง","รายละเอียด"';

function row(seq: string, kind: string, ref: string, at: string, actor: string, detail: string) {
  return makeDeviceEventRow({ seq, kind, ref, at, actor, detail });
}

beforeEach(() => {
  setLang('th');
});

describe('eventsCsv (#404 D17)', () => {
  it('no rows: the BOM and the header only, with no line break after it', () => {
    expect(eventsCsv([])).toBe(BOM + HEADER);
  });

  it('one line per row in the order given: at, actor, raw kind, ref, detail; quoted; CRLF between lines', () => {
    const rows = [
      row('42', 'PRINT_RECEIPT', 'TX-2', '2026-10-08T03:04:05.000Z', 'front1', '{"jobId":"J-3"}'),
      row('41', 'DRAWER_MANUAL_OBSERVATION', '', '2026-10-08T03:00:00.123Z', 'backend1', '{"state":"OPEN"}'),
    ];
    expect(eventsCsv(rows)).toBe(
      BOM +
        HEADER +
        '\r\n' +
        '"2026-10-08T03:04:05.000Z","front1","PRINT_RECEIPT","TX-2","{""jobId"":""J-3""}"' +
        '\r\n' +
        '"2026-10-08T03:00:00.123Z","backend1","DRAWER_MANUAL_OBSERVATION","","{""state"":""OPEN""}"',
    );
  });

  it('a cell starting with =, +, @ or - is prefixed with a quote mark, in every column (§9)', () => {
    const rows = [row('7', '=KIND', '+66812345678', '-1', '@actor', '=HYPERLINK("http://x")')];
    expect(eventsCsv(rows)).toBe(
      BOM + HEADER + '\r\n' + `"'-1","'@actor","'=KIND","'+66812345678","'=HYPERLINK(""http://x"")"`,
    );
  });

  it('only the first character counts: = + @ - later in a cell, and other leading characters, are left alone', () => {
    const rows = [row('8', 'A=B', 'x-1', '2026-10-08T03:04:05.000Z', 'a@b', ' =spaced')];
    expect(eventsCsv(rows)).toBe(
      BOM + HEADER + '\r\n' + '"2026-10-08T03:04:05.000Z","a@b","A=B","x-1"," =spaced"',
    );
  });

  it('the seq column is not exported, and the rows are not re-ordered by it', () => {
    const rows = [
      row('1', 'K1', 'R1', 'T1', 'A1', 'D1'),
      row('9', 'K9', 'R9', 'T9', 'A9', 'D9'),
    ];
    const lines = eventsCsv(rows).split('\r\n');
    expect(lines).toEqual([BOM + HEADER, '"T1","A1","K1","R1","D1"', '"T9","A9","K9","R9","D9"']);
  });
});
