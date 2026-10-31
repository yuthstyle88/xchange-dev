// /admin/pricing-formula (docs/design/blueprint/pricing-formula-web.md §1; pricing-formula.md §6 FE row, F-1…F-4):
// the organisation's AUTO/FOLLOW price formula. Shown to a session holding `PricingFormulaSave`, the permission
// `pricingFormulaSave` and the `ORGANISATION` `pricingFormulaPreview` ask for (§5, F-2).
import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { PricingFormula } from './PricingFormula';
import * as table from './i18n';

const t = screenT('admin.pricing-formula', table);

export const menu: AdminMenu = {
  id: 'pricing-formula',
  order: 30,
  group: 'money',
  needs: 'PricingFormulaSave',
  dated: false,
  icon: 'ƒ',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: PricingFormula,
  blueprint: true,
};
