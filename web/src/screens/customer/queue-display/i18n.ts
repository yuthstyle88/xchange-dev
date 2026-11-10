export const th = {
  nowCalling: 'กำลังเรียกคิว',
  recentCalled: 'เรียกแล้วล่าสุด',
  empty: 'ยังไม่มีคิวที่เรียก',
  fullscreen: 'เต็มจอ',
  soundOn: 'เปิดเสียงเรียกคิว',
  connecting: 'กำลังเชื่อมต่อ…',
  stale: 'ขาดการเชื่อมต่อ ข้อมูลอาจไม่เป็นปัจจุบัน',
  lastUpdated: 'อัปเดตล่าสุด',
  neverUpdated: 'ยังไม่ได้รับข้อมูล',
};

export const en: Record<keyof typeof th, string> = {
  nowCalling: 'Now calling',
  recentCalled: 'Recently called',
  empty: 'No called numbers',
  fullscreen: 'Fullscreen',
  soundOn: 'Turn on call sound',
  connecting: 'Connecting…',
  stale: 'Connection lost, numbers may be out of date',
  lastUpdated: 'Last updated',
  neverUpdated: 'No data received yet',
};

export const zh: Record<keyof typeof th, string> = {
  nowCalling: '正在叫号',
  recentCalled: '最近叫号',
  empty: '暂无叫号',
  fullscreen: '全屏',
  soundOn: '开启叫号声音',
  connecting: '正在连接…',
  stale: '连接中断，信息可能不是最新',
  lastUpdated: '最后更新',
  neverUpdated: '尚未收到数据',
};
