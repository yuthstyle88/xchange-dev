// P01 v2 fixed design text (rate-display.md §9.1, rulings on PR 3558: TH · EN on the board, 中文 in the ticker).
// Every label of the board lives here; components never hold a label as a literal. Each label is fixed text,
// not a language switch: the board renders `th · en` as the reference example does
// (docs/design/reference/rate-board-example/rate-board.body.html).

export const BOARD_TEXT = {
  brand: 'XCHANGE',
  brandSub: 'CURRENCY EXCHANGE · BOARDING PASS',
  from: { th: 'ต้นทาง', en: 'FROM' },
  fromCurrency: { code: 'THB', name: 'Thai Baht' },
  to: { th: 'ปลายทาง', en: 'TO' },
  currencies: { th: 'สกุลเงิน', en: 'currencies' },
  branch: { th: 'สาขา', en: 'BRANCH' },
  status: { th: 'สถานะ', en: 'STATUS' },
  synced: { th: 'เชื่อมต่อข้อมูลสาขาแล้ว', en: 'Synced with counter' },
  delayed: { th: 'ข้อมูลล่าช้า · โปรดยืนยัน', en: 'Delayed / confirm at counter' },
  offline: { th: 'งดแสดงเรท · เชื่อมต่อไม่ได้', en: 'Offline / no rates displayed' },
  staleReference: { th: 'เรทอ้างอิงเก่า', en: 'Reference rate is old' },
  currencyHeading: { th: 'สกุลเงิน', en: 'Currency' },
  bandSub: { th: 'รับซื้อ · ขาย', en: 'Buy · Sell' },
  allAmounts: { th: 'ทุกยอด', en: 'All amounts' },
  loading: 'กำลังโหลดเรทจากสาขา…',
  askCounter: 'กรุณาสอบถามเรทที่เคาน์เตอร์ · Please ask at the counter',
  noCurrencies: 'สาขานี้ยังไม่มีสกุลเงินที่เปิดให้บริการ',
  stubTitle: 'BEYOND CURRENCIES · EVERY TRIP & JOURNEY',
  stubConditions: [
    'ราคาตามยอดแลกรวมเป็นบาท · Rate by total THB amount',
    'เรทจากหน้าทำรายการสาขา กรุณายืนยันก่อนแลกเงิน · Confirm final rates at the counter',
  ],
  synced_at: { th: 'ตรวจข้อมูล', en: 'SYNCED' },
  sourceDate: { th: 'วันที่เรทอ้างอิง', en: 'SOURCE' },
  snapshot: { th: '', en: 'SNAPSHOT' },
  hours: { th: 'เวลาทำการ', en: 'OPEN' },
  tickerGreetings: ['ยินดีต้อนรับ', 'WELCOME', '欢迎光临', 'ようこそ', '환영합니다', 'ДОБРО ПОЖАЛОВАТЬ'],
  tickerExchangeHere: 'แลกเงินได้ที่เคาน์เตอร์นี้ · EXCHANGE HERE · 在此兑换',
  tickerCount: { th: 'สกุลเงิน', en: 'CURRENCIES', zh: '种货币' },
  tickerBetterRates: 'ยิ่งแลกมาก เรทยิ่งดี · BETTER RATES FOR LARGER AMOUNTS · 兑换越多汇率越好',
} as const;

/** `th · en`, the board's way of showing a fixed bilingual label. */
export function bilingual(label: { th: string; en: string }): string {
  return label.th ? `${label.th} · ${label.en}` : label.en;
}
