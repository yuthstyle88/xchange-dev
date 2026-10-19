// B06 board texts (TH/EN parity). Filled by the board's ticket.
export const th = {
  loading: 'กำลังโหลด...',
  offline: 'เชื่อมต่อไม่ได้ · กำลังลองใหม่',
  waiting: 'รอเรียก',
  emptyWaiting: 'ไม่มีคิวรอ',
  nowServing: 'กำลังให้บริการ',
  emptyCalled: 'ไม่มีคิวที่กำลังให้บริการ',
  booking: 'BOOKING',
  walkIn: 'WALK_IN',
  called: 'CALLED',
  serving: 'SERVING',
  mine: 'ของฉัน',
};

export const en: typeof th = {
  loading: 'Loading...',
  offline: 'Cannot connect · Retrying',
  waiting: 'Waiting',
  emptyWaiting: 'No waiting tickets',
  nowServing: 'Now serving',
  emptyCalled: 'No tickets being served',
  booking: 'BOOKING',
  walkIn: 'WALK_IN',
  called: 'CALLED',
  serving: 'Serving',
  mine: 'Mine',
};
