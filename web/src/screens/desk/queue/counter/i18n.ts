// B06 counter bar texts (TH/EN parity). Filled by the counter bar's ticket.
export const th = {
  issueWalkIn: 'ออกบัตรคิวหน้าร้าน',
  callNext: 'เรียกคิวถัดไป',
  pause: 'พักเคาน์เตอร์',
  resume: 'เปิดรับคิวต่อ',
  openQueue: 'เปิดรับคิวสาขา',
  closeQueue: 'ปิดรับคิวสาขา',
  noCounter: 'ยังไม่ได้เปิดกะ เคาน์เตอร์ยังไม่พร้อมเรียกคิว',
  'error.nothingWaiting': 'ไม่มีคิวที่รออยู่',
  'error.queueClosed': 'คิวสาขาปิดอยู่',
  'error.noOpenShift': 'ยังไม่ได้เปิดกะ กรุณาเปิดกะก่อน',
  'error.counterPaused': 'เคาน์เตอร์พักอยู่ กรุณาเปิดรับคิวก่อน',
};

export const en: typeof th = {
  issueWalkIn: 'Issue walk-in ticket',
  callNext: 'Call next',
  pause: 'Pause counter',
  resume: 'Resume counter',
  openQueue: 'Open branch queue',
  closeQueue: 'Close branch queue',
  noCounter: 'No open shift; counter cannot call tickets yet',
  'error.nothingWaiting': 'No waiting tickets in queue',
  'error.queueClosed': 'Branch queue is closed',
  'error.noOpenShift': 'No open shift; open a shift first',
  'error.counterPaused': 'Counter is paused; resume before calling',
};
