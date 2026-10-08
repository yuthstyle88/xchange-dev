export const th = {
  title: 'เข้าสู่ระบบพนักงาน',
  username: 'ชื่อผู้ใช้',
  password: 'รหัสผ่าน',
  submit: 'เข้าสู่ระบบ',
} as const;

export const en: Record<keyof typeof th, string> = {
  title: 'Staff Sign-In',
  username: 'Username',
  password: 'Password',
  submit: 'Sign In',
};
