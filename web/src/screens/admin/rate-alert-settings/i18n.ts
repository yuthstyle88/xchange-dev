export const th = {
  label: 'ตั้งค่าแจ้งเตือนเรท',
  eyebrow: 'EXECUTIVE SETTINGS',
  title: 'ตั้งค่าการแจ้งเตือนเรท (Rate Alerts)',
  subtitle: 'กำหนดค่าจำกัดการแจ้งเตือนเรทสำหรับลูกค้าและนโยบายการเฝ้าดูเรท',

  loading: 'กำลังโหลดข้อมูลการตั้งค่า...',
  retry: 'ลองใหม่',
  save: 'บันทึกการตั้งค่า',
  saving: 'กำลังบันทึก...',
  saved: 'บันทึกการตั้งค่าสำเร็จ',

  emptyTitle: 'ยังไม่มีการบันทึกการตั้งค่าเริ่มต้น',
  emptyNote: 'ระบบแจ้งเตือนเรทปิดใช้งานจนกว่าจะบันทึกการตั้งค่านี้ ลูกค้ายังไม่สามารถสร้างการแจ้งเตือนได้ (แสดงค่าแนะนำตาม Blueprint)',
  suggestedValues: 'ค่าแนะนำตาม Blueprint',
  suggested: 'แนะนำ',
  useSuggested: 'ใช้ค่าแนะนำ',

  cooldownLabel: 'ระยะเวลา Cooldown ขั้นต่ำ (ชั่วโมง)',
  cooldownHint: 'ระยะเวลารอขั้นต่ำหลังเกิดการแจ้งเตือน ก่อนที่จะแจ้งเตือนครั้งถัดไป (แปลงเป็นวินาทีเมื่อส่ง)',
  maxActiveLabel: 'จำนวนการแจ้งเตือนสูงสุดต่อลูกค้า (รายการ)',
  maxActiveHint: 'จำนวน Alert ที่ทำงานอยู่พร้อมกันได้สูงสุดต่อคน (รวมที่หยุดชั่วคราว)',
  lifetimeLabel: 'อายุการแจ้งเตือนเริ่มต้น (วัน)',
  lifetimeHint: 'ระยะเวลาหมดอายุเริ่มต้นของ Alert (เว้นว่างไว้ได้หากไม่ต้องการกำหนดวันหมดอายุ)',
  observePolicyLabel: 'นโยบายการเฝ้าดูเรท (Observe Policy)',

  policyAllFreshTitle: 'ALL_FRESH',
  policyAllFreshDesc: 'ทุกเรทที่ยังสด (ทั้ง AUTO และตั้งเอง แต่ข้ามเรทที่ค้าง/หมดอายุ)',
  policyAllTitle: 'ALL',
  policyAllDesc: 'ทุกเรท (รวมเรทที่ค้าง/หมดอายุ)',
  policyAutoFreshTitle: 'AUTO_FRESH',
  policyAutoFreshDesc: 'เฉพาะเรท AUTO ที่ยังสด (ข้ามเรทที่ตั้งเอง)',

  errorRequired: 'กรุณากรอกข้อมูลให้ครบถ้วน',
  errorCooldownInvalid: 'ระยะเวลา Cooldown ต้องเป็นตัวเลขบวกที่มากกว่า 0',
  errorMaxActiveInvalid: 'จำนวนการแจ้งเตือนสูงสุดต้องเป็นตัวเลขจำนวนเต็มบวกที่มากกว่า 0',
  errorLifetimeInvalid: 'อายุการแจ้งเตือนต้องเป็นตัวเลขจำนวนเต็มบวกที่มากกว่า 0 หรือเว้นว่างไว้',
  errorPolicyRequired: 'กรุณาเลือกนโยบายการเฝ้าดูเรท (Observe Policy)',
};

export const en = {
  label: 'Rate Alerts',
  eyebrow: 'EXECUTIVE SETTINGS',
  title: 'Rate Alert Settings',
  subtitle: 'Configure rate alert limits for customers and rate observation policy',

  loading: 'Loading settings...',
  retry: 'Retry',
  save: 'Save Settings',
  saving: 'Saving...',
  saved: 'Settings saved successfully',

  emptyTitle: 'No Initial Settings Configured',
  emptyNote: 'Rate alerts are closed until settings are saved. Customers cannot create alerts until this screen is saved. (Suggested Blueprint values are shown)',
  suggestedValues: 'Blueprint Suggested Values',
  suggested: 'Suggested',
  useSuggested: 'Use Suggested Values',

  cooldownLabel: 'Default Cooldown (hours)',
  cooldownHint: 'Minimum cooldown after an alert fires before notifying again (sent in seconds)',
  maxActiveLabel: 'Max Active Alerts per Customer',
  maxActiveHint: 'Maximum active alerts allowed per customer (including paused alerts)',
  lifetimeLabel: 'Default Lifetime (days)',
  lifetimeHint: 'Default expiration period for alerts (leave blank for no expiry)',
  observePolicyLabel: 'Rate Observation Policy',

  policyAllFreshTitle: 'ALL_FRESH',
  policyAllFreshDesc: 'All fresh rates (both AUTO and manual, skipping stale rates)',
  policyAllTitle: 'ALL',
  policyAllDesc: 'All rates (including stale rates)',
  policyAutoFreshTitle: 'AUTO_FRESH',
  policyAutoFreshDesc: 'Only fresh AUTO rates (skipping manual rates)',

  errorRequired: 'Please fill in all required fields',
  errorCooldownInvalid: 'Cooldown must be a positive number greater than 0',
  errorMaxActiveInvalid: 'Max active alerts must be a positive integer greater than 0',
  errorLifetimeInvalid: 'Lifetime must be a positive integer greater than 0 or left blank',
  errorPolicyRequired: 'Please select a rate observation policy',
};
