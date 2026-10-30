// #5733 (owner ruling A on #5683; pricing-formula.md §5 F-2): the formula screen is shown to a session whose group
// holds `PricingFormulaSave`, the permission `pricingFormulaSave` and the ORGANISATION preview ask for. A group with
// only `RatePolicySave` (RATE_MANAGER's seed) would see the menu and then be refused on save.
// Written before the implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken;
// add tests in a file of your own.
import { describe, expect, it } from 'vitest';
import { menu } from './menu';

describe('#5733 pricing-formula menu', () => {
  it('needs PricingFormulaSave, not RatePolicySave', () => {
    expect(menu.needs).toBe('PricingFormulaSave');
  });
});
