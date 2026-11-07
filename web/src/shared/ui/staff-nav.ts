export type StaffNavItem = {
  key: string;
  href: string;
  permission?: string;
  icon: string;
};

export const STAFF_NAV: StaffNavItem[] = [
  { key: 'shell.nav.counter', href: '/desk', permission: 'ExchangeTransact', icon: '⇄' },
  { key: 'shell.nav.backdated', href: '/desk/backdated', permission: 'exchangeBackdate', icon: '↺' },
  { key: 'shell.nav.vault', href: '/#cash', permission: 'CashManage', icon: '▣' },
  { key: 'shell.nav.reports', href: '/#reports', permission: 'CompatRead', icon: '≡' },
];

export function visibleStaffNav(permissions: readonly string[]): StaffNavItem[] {
  return STAFF_NAV.filter((i) => !i.permission || permissions.includes(i.permission));
}
