// #998 APP-7: a screen folder segment may start with one underscore, so the customer app's private shared folder
// `screens/customer/_shell` is a screen whose keys live under `customer._shell.*` (FE lane controller ruling,
// issuecomment-6138295640). Everything the segment rule refused before is still refused.
import { describe, expect, it } from 'vitest';
import { collectScreens } from './index';

function customer(key: string) {
  return { th: { [key]: 'ก' }, en: { [key]: 'A' }, zh: { [key]: '甲' } };
}

describe('screen folder segments (#998)', () => {
  it('accepts `_shell`, namespaced as customer._shell', () => {
    const tables = collectScreens({ '/src/screens/customer/_shell/i18n.ts': customer('home') });
    expect(tables.th['customer._shell.home']).toBe('ก');
    expect(tables.en['customer._shell.home']).toBe('A');
    expect(tables.zh['customer._shell.home']).toBe('甲');
  });

  it.each(['__x', '_9x', '_', 'x_'])('refuses the segment %s', (segment) => {
    const path = `/src/screens/customer/${segment}/i18n.ts`;
    expect(() => collectScreens({ [path]: customer('a') }), path).toThrow(/Invalid screen path segment/);
  });

  it.each(['Admin_X', 'pay_ments', '9pay', '-pay'])('still refuses %s', (segment) => {
    const path = `/src/screens/admin/${segment}/i18n.ts`;
    expect(() => collectScreens({ [path]: { th: { a: 'ก' }, en: { a: 'A' } } }), path).toThrow(
      /Invalid screen path segment/,
    );
  });
});
