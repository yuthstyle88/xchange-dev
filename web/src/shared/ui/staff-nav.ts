export type SettingsGroup = 'cash' | 'alerts';

export type SettingsLinkKey = 'cashLimits' | 'cashPolicy' | 'alertSettings' | 'rateAlertSettings';

export type SettingsLinkItem = {
  group: SettingsGroup;
  key: SettingsLinkKey;
  href: string;
  permission: string;
};

export const SETTINGS_LINKS: readonly SettingsLinkItem[] = [
  { group: 'cash', key: 'cashLimits', href: '/#cash-limits', permission: 'settingsRead' },
  { group: 'cash', key: 'cashPolicy', href: '/#cash-policy', permission: 'settingsRead' },
  { group: 'alerts', key: 'alertSettings', href: '/#alert-settings', permission: 'AlertSettingsSave' },
  { group: 'alerts', key: 'rateAlertSettings', href: '/#rate-alert-settings', permission: 'RateAlertSettingsSave' },
];

export function visibleSettingsLinks(permissions: readonly string[]): SettingsLinkItem[] {
  return SETTINGS_LINKS.filter((item) => permissions.includes(item.permission));
}

export function hasVisibleSettings(permissions: readonly string[]): boolean {
  return visibleSettingsLinks(permissions).length > 0;
}

export type StaffNavItem = {
  key: string;
  href: string;
  permission?: string;
  visible?: (permissions: readonly string[]) => boolean;
  icon: string;
};

export const STAFF_NAV: StaffNavItem[] = [
  { key: 'shell.nav.counter', href: '/desk', permission: 'ExchangeTransact', icon: '⇄' },
  { key: 'shell.nav.backdated', href: '/desk/backdated', permission: 'exchangeBackdate', icon: '↺' },
  { key: 'shell.nav.vault', href: '/#cash', permission: 'CashManage', icon: '▣' },
  { key: 'shell.nav.reports', href: '/#reports', permission: 'CompatRead', icon: '≡' },
  { key: 'shell.nav.settings', href: '/staff/settings', visible: hasVisibleSettings, icon: '⚙' },
];

export function visibleStaffNav(permissions: readonly string[]): StaffNavItem[] {
  return STAFF_NAV.filter((i) => (i.visible ? i.visible(permissions) : !i.permission || permissions.includes(i.permission)));
}
