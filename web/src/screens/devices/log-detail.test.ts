// OL-8 device-log-detail (verifier row 98, ~/108-Ting-Ecosystem/.worktrees/xc-verify-r12/VERIFY.md): /devices still
// prints the device log's detail as raw JSON ({"state":"OPEN","agentId":"5fe9d9ce-…"}), event kinds with no label as
// their raw code, and full UUIDs. This file pins the pure half: `log-detail.ts` (`shortId`, `detailLines`), the five
// kinds the API writes that `labels.ts` has no label for, the detail key and value labels, and the `.dev-detail` list
// rule in `devices.css`. The screen half is DevicesScreen.log-detail.test.tsx.
// Written before the implementation by claude/01DKFAnW (pane claude-15) (round 13, sub-controller claude/1cc51615). Do not edit, delete or
// weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { setLang } from '../../shared/i18n';
import { formatBangkokDateTime } from '../../shared/format/bangkok-time';
import { DETAIL_KEY_LABELS, KIND_LABELS, kindLabel } from './labels';
import { detailLines, shortId } from './log-detail';

const UUID = '5fe9d9ce-5400-4000-8000-000000000001';

beforeEach(() => {
  setLang('th');
});

afterEach(() => {
  setLang('th');
});

describe('shortId', () => {
  it('LD-1 a UUID shows its first 8 characters', () => {
    expect(shortId(UUID)).toBe('5fe9d9ce');
    expect(shortId(UUID.toUpperCase())).toBe('5FE9D9CE');
  });

  it('LD-2 a prefixed UUID keeps its prefix', () => {
    expect(shortId(`agent:${UUID}`)).toBe('agent:5fe9d9ce');
  });

  it('LD-3 anything that is not a whole UUID is left as it is', () => {
    for (const v of ['', 'J3', 'TX-2', 'XC-20261105-000023', 'teller1', `${UUID} x`, '5fe9d9ce-5400']) {
      expect(shortId(v), v).toBe(v);
    }
  });
});

describe('detailLines', () => {
  it("LD-4 the verifier's line: known keys labelled, the value word labelled, the UUID shortened with its full id kept", () => {
    expect(detailLines(`{"state":"OPEN","agentId":"${UUID}"}`)).toEqual([
      { label: 'สถานะ', text: 'เปิด' },
      { label: 'device agent', text: '5fe9d9ce', full: UUID },
    ]);
  });

  it('LD-5 unknown keys are still shown, under their own name, in the order sent', () => {
    expect(detailLines('{"zzz":"abc","a":1,"reason":"ส่งมอบ"}')).toEqual([
      { label: 'zzz', text: 'abc' },
      { label: 'a', text: '1' },
      { label: 'เหตุผล', text: 'ส่งมอบ' },
    ]);
  });

  it('LD-6 yes/no for booleans, — for null, compact JSON for a nested value', () => {
    expect(detailLines('{"physicalAction":false,"sensorConfirmed":true,"cupsJob":null,"x":{"y":1}}')).toEqual([
      { label: 'สั่งอุปกรณ์จริง', text: 'ไม่ใช่' },
      { label: 'เซนเซอร์ยืนยัน', text: 'ใช่' },
      { label: 'งานพิมพ์ CUPS', text: '—' },
      { label: 'x', text: '{"y":1}' },
    ]);
  });

  it('LD-7 the words of state, source and mode are labelled; other keys keep their words', () => {
    expect(
      detailLines('{"state":"CLOSED","source":"EMPLOYEE_OBSERVATION","mode":"SIMULATION","reason":"OPEN"}'),
    ).toEqual([
      { label: 'สถานะ', text: 'ปิด' },
      { label: 'ที่มา', text: 'พนักงานรายงาน' },
      { label: 'โหมด', text: 'จำลอง' },
      { label: 'เหตุผล', text: 'OPEN' },
    ]);
  });

  it('LD-8 the time keys are Bangkok date-times', () => {
    const at = '2026-10-08T02:58:00.000Z';
    expect(detailLines(`{"queuedAt":"${at}","leasedAt":"${at}","reportedAt":"${at}"}`)).toEqual([
      { label: 'เวลาสั่งงาน', text: formatBangkokDateTime(at) },
      { label: 'เวลารับงาน', text: formatBangkokDateTime(at) },
      { label: 'เวลารายงาน', text: formatBangkokDateTime(at) },
    ]);
  });

  it('LD-9 an empty detail is one line —', () => {
    expect(detailLines('{}')).toEqual([{ label: '', text: '—' }]);
    expect(detailLines('')).toEqual([{ label: '', text: '—' }]);
  });

  it('LD-10 a detail that is not a JSON object is kept whole, as one unlabelled line', () => {
    expect(detailLines('not json')).toEqual([{ label: '', text: 'not json' }]);
    expect(detailLines('[1,2]')).toEqual([{ label: '', text: '[1,2]' }]);
    expect(detailLines('"word"')).toEqual([{ label: '', text: '"word"' }]);
  });

  it('LD-11 English labels when the language is English', () => {
    setLang('en');
    expect(detailLines('{"state":"OPEN","physicalAction":true}')).toEqual([
      { label: 'State', text: 'Open' },
      { label: 'Physical action', text: 'Yes' },
    ]);
  });
});

describe('labels for the device log', () => {
  it('LD-12 the five kinds the API writes to device_events with no label until now have their Thai label', () => {
    // api/src/infrastructure/device_agents/claim.rs:105, :235; devices/device_service.rs:367;
    // crates/xc-kiosk/src/domain/kiosk/value_objects/audit_line.rs:59-60.
    const expected: Record<string, string> = {
      JOB_LEASED: 'device agent รับงาน',
      AGENT_BRANCH_MISMATCH: 'device agent ผิดสาขา',
      DRAWER_SKIPPED: 'ข้ามการเปิดลิ้นชัก (สาขาไม่มีลิ้นชัก)',
      KIOSK_CANCELLED: 'ยกเลิกคิว kiosk',
      KIOSK_CUSTOMER_DIFFERS: 'ลูกค้าที่บันทึกไม่ตรงกับคิว kiosk',
    };
    for (const [code, label] of Object.entries(expected)) {
      expect(Object.prototype.hasOwnProperty.call(KIND_LABELS, code), code).toBe(true);
      expect(kindLabel(code), code).toBe(label);
    }
  });

  it('LD-13 an unknown kind still shows its code', () => {
    expect(kindLabel('DRAWER_OBSERVED')).toBe('DRAWER_OBSERVED');
  });

  it('LD-14 every detail key the API writes has its Thai label', () => {
    const expected: Record<string, string> = {
      job: 'งาน',
      jobId: 'งาน',
      reason: 'เหตุผล',
      mode: 'โหมด',
      physicalAction: 'สั่งอุปกรณ์จริง',
      physicalOpenConfirmed: 'ยืนยันเปิดจริง',
      sensorConfirmed: 'เซนเซอร์ยืนยัน',
      state: 'สถานะ',
      source: 'ที่มา',
      online: 'ออนไลน์',
      slip: 'สลิป',
      mime: 'ชนิดไฟล์',
      submissionId: 'เลขที่ส่ง',
      passport: 'พาสปอร์ต',
      currency: 'สกุลเงิน',
      leaseId: 'เลขการรับงาน',
      namedBranch: 'สาขาที่ระบุ',
      queuedBy: 'ผู้สั่งงาน',
      queuedAt: 'เวลาสั่งงาน',
      agent: 'device agent',
      agentId: 'device agent',
      agentName: 'ชื่อ device agent',
      agentVersion: 'รุ่น device agent',
      drawerTarget: 'ลิ้นชักที่สั่ง',
      leasedAt: 'เวลารับงาน',
      reportedAt: 'เวลารายงาน',
      cupsJob: 'งานพิมพ์ CUPS',
      foreignAmount: 'ยอดต่างประเทศ',
      thb: 'ยอด THB',
      rate: 'เรท',
      type: 'ประเภท',
      note: 'หมายเหตุ',
      paymentMethod: 'วิธีชำระ',
      documentRef: 'เลขเอกสาร',
      requestCustomerId: 'ลูกค้าในคำขอ',
      savedCustomerId: 'ลูกค้าที่บันทึก',
    };
    expect(Object.keys(DETAIL_KEY_LABELS).sort()).toEqual(Object.keys(expected).sort());
    for (const [key, label] of Object.entries(expected)) {
      expect(detailLines(JSON.stringify({ [key]: 'v' }))[0]?.label, key).toBe(label);
    }
  });
});

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let readHere: (name: string) => string = () => '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  readHere = (name) => fs.readFileSync(here + name, 'utf8');
});

function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = css.replace(/\/\*[\s\S]*?\*\//g, '');
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    return out;
  }
  return undefined;
}

describe('the detail list look', () => {
  it('LD-15 .dev-detail is a bare list; its key is muted', () => {
    const css = readHere('devices.css');
    const list = decls(css, '.xc-devices .xc-shell-main .dev-detail');
    expect(list?.get('list-style')).toBe('none');
    expect(list?.get('margin')).toBe('0');
    expect(list?.get('padding')).toBe('0');
    expect(list?.get('font-size')).toBe('13px');
    expect(decls(css, '.xc-devices .xc-shell-main .dev-detail-key')?.get('color')).toBe('var(--xc-muted)');
  });
});
