export const th = {
  title: 'ทำรายการแลกเปลี่ยนเงิน',
  stepsLabel: 'ขั้นตอนทำรายการ',
  empty: 'ยังไม่มีขั้นตอนทำรายการ',
} as const;

export const en: Record<keyof typeof th, string> = {
  title: 'Money Exchange',
  stepsLabel: 'Transaction steps',
  empty: 'No steps available',
};
