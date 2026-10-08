// V2-parts (#476): the label maps of an exchange (ticket §7 `transaction.ts`, design §12.2).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`, decoded admin page): `typeName` and `methodName` at
// `admin-html:56` — anything that is not `BUY` is `ร้านขาย`, an unknown method is `อื่น ๆ`;
// the status words at `admin-html:63` — anything that is neither `COMPLETED` nor `CANCELLED` is
// `รอบันทึกเอกสาร`. Thai through `parts/i18n.ts` (W10).
import { afterEach, describe, expect, it } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { methodName, statusName, typeName } from './transaction';

const THAI = /[฀-๿]/;

const METHODS: Array<[string, string]> = [
  ['Cash', 'เงินสด'],
  ['Credit Card', 'บัตรเครดิต'],
  ['Alipay', 'Alipay'],
  ['PromptPay', 'พร้อมเพย์'],
  ['Bank Transfer', 'โอนธนาคาร'],
  ['SCB Transfer', 'โอนธนาคาร'],
];

afterEach(() => {
  setLang('th');
});

describe('typeName, methodName, statusName (#476 P1)', () => {
  it("typeName: BUY is ร้านรับซื้อ, anything else ร้านขาย (admin-html:56)", () => {
    setLang('th');
    expect(typeName('BUY')).toBe('ร้านรับซื้อ');
    expect(typeName('SELL')).toBe('ร้านขาย');
    expect(typeName('')).toBe('ร้านขาย');
  });

  it("methodName: v2's six methods, and อื่น ๆ for any other (admin-html:56)", () => {
    setLang('th');
    for (const [method, label] of METHODS) {
      expect(methodName(method), method).toBe(label);
    }
    expect(methodName('Crypto')).toBe('อื่น ๆ');
    expect(methodName('')).toBe('อื่น ๆ');
    expect(methodName('cash'), 'the method code is matched exactly').toBe('อื่น ๆ');
  });

  it("statusName: สำเร็จ, ยกเลิก, and รอบันทึกเอกสาร for anything else (admin-html:63)", () => {
    setLang('th');
    expect(statusName('COMPLETED')).toBe('สำเร็จ');
    expect(statusName('CANCELLED')).toBe('ยกเลิก');
    expect(statusName('PENDING')).toBe('รอบันทึกเอกสาร');
    expect(statusName('')).toBe('รอบันทึกเอกสาร');
  });

  it('in English every label switches, and the two types and three statuses stay distinct (W10)', () => {
    setLang('en');
    const labels = [
      typeName('BUY'),
      typeName('SELL'),
      statusName('COMPLETED'),
      statusName('CANCELLED'),
      statusName('PENDING'),
      ...METHODS.map(([m]) => methodName(m)),
      methodName('Crypto'),
    ];
    for (const label of labels) {
      expect(label.length, label).toBeGreaterThan(0);
      expect(THAI.test(label), `${label}: no Thai in English`).toBe(false);
    }
    expect(typeName('BUY')).not.toBe(typeName('SELL'));
    expect(new Set([statusName('COMPLETED'), statusName('CANCELLED'), statusName('PENDING')]).size).toBe(3);
    expect(methodName('Bank Transfer')).toBe(methodName('SCB Transfer'));
    expect(methodName('Cash')).not.toBe(methodName('Crypto'));
  });
});
