// V1-devices-screen (#404): the page's current-behaviour text (ticket §7 `banner.ts`, §10.3 V6).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// V6, from the `devices` view (#400 §7.1):
// - banner: `printerKind` SIMULATED → bannerSimulated; NETWORK → bannerNetwork.
// - drawer: `drawerAgentActive` → drawerAgent; else NETWORK with `hasDrawer` not `false` →
//   drawerNetwork with the printer model; else drawerNone.
// The texts are the screen's table (`i18n.test.ts` pins the Thai); they follow the language.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { DevicesOverview } from '../../shared/api/generated/devices';
import { fill, setLang } from '../../shared/i18n';
import { deviceNotice } from './banner';
import { en, th } from './i18n';

function overview(
  printerKind: string,
  drawerAgentActive: boolean,
  hasDrawer: boolean | null,
  printerModel = 'EPSON TM-T82X',
): DevicesOverview {
  return {
    branchId: 'B1',
    branches: [{ id: 'B1', name: 'สาขาหนึ่ง' }],
    printerOnline: true,
    printerKind,
    printerModel,
    hasDrawer,
    drawerAgentActive,
    balanceThb: '0.00',
    balanceUsd: '0.00',
    completedCount: 0,
  };
}

beforeEach(() => setLang('th'));
afterEach(() => setLang('th'));

describe('deviceNotice (#404 V6)', () => {
  it.each([
    // kind, agent, hasDrawer, banner key, drawer key
    ['SIMULATED', false, null, 'bannerSimulated', 'drawerNone'],
    ['SIMULATED', false, true, 'bannerSimulated', 'drawerNone'],
    ['SIMULATED', true, null, 'bannerSimulated', 'drawerAgent'],
    ['NETWORK', false, true, 'bannerNetwork', 'drawerNetwork'],
    ['NETWORK', false, null, 'bannerNetwork', 'drawerNetwork'],
    ['NETWORK', false, false, 'bannerNetwork', 'drawerNone'],
    ['NETWORK', true, false, 'bannerNetwork', 'drawerAgent'],
    ['NETWORK', true, true, 'bannerNetwork', 'drawerAgent'],
  ] as const)('%s, agent %s, hasDrawer %s → %s / %s', (kind, agent, hasDrawer, bannerKey, drawerKey) => {
    const o = overview(kind, agent, hasDrawer);
    const expectedDrawer =
      drawerKey === 'drawerNetwork' ? fill(th.drawerNetwork, { printerModel: o.printerModel }) : th[drawerKey];
    expect(deviceNotice(o)).toEqual({ banner: th[bannerKey], drawer: expectedDrawer });
  });

  it("names the view's printer model, as sent", () => {
    const o = overview('NETWORK', false, true, 'NEOCAL NTP-8220BT (LAN 2)');
    expect(deviceNotice(o).drawer).toBe(fill(th.drawerNetwork, { printerModel: 'NEOCAL NTP-8220BT (LAN 2)' }));
    expect(deviceNotice(o).drawer).toContain('NEOCAL NTP-8220BT (LAN 2)');
  });

  it('follows the language', () => {
    setLang('en');
    expect(deviceNotice(overview('SIMULATED', false, null))).toEqual({ banner: en.bannerSimulated, drawer: en.drawerNone });
    expect(deviceNotice(overview('NETWORK', false, null, 'M1'))).toEqual({
      banner: en.bannerNetwork,
      drawer: fill(en.drawerNetwork, { printerModel: 'M1' }),
    });
    expect(deviceNotice(overview('NETWORK', true, null))).toEqual({ banner: en.bannerNetwork, drawer: en.drawerAgent });
  });
});
