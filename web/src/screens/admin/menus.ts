// V2-shell (#460): admin menu types and discovery (§7, §9; design §3.3, §12.3)

import type { JSX } from 'solid-js';

export interface AdminMenuProps {
  branch: () => string;
  period: () => { from: string; to: string };
  shown: () => boolean;
}

export interface AdminMenuHeading {
  eyebrow: () => string;
  title: () => string;
  subtitle: () => string;
}

export interface AdminMenu {
  id: string;
  label: () => string;
  icon: string;
  order: number;
  group: 'desk' | 'money' | 'data' | 'tools';
  /** `'all'`, or the `Permission` name a user's `me.permissions` must hold (#1030 §3.10). */
  needs: 'all' | string;
  heading: AdminMenuHeading;
  dated: boolean;
  Component: (props: AdminMenuProps) => JSX.Element;
  blueprint?: true;
}

const ID_SHAPE = /^[a-z][a-z0-9-]*$/;

export function collectMenus(
  globResult: Record<string, { menu: AdminMenu }>,
): AdminMenu[] {
  const result: AdminMenu[] = [];
  const seenIds = new Set<string>();
  const seenOrders = new Set<number>();

  for (const [key, value] of Object.entries(globResult)) {
    if (!value || !value.menu) {
      continue;
    }
    const menu = value.menu;

    // extract folder name from ./<folder>/menu.ts
    const match = key.match(/^\.\/([^/]+)\/menu\.ts$/);
    const folder = match ? match[1] : null;

    if (!folder || folder !== menu.id) {
      throw new Error(
        `Menu id '${menu.id}' must match its folder name '${folder}' in '${key}'`,
      );
    }

    if (menu.id === 'parts') {
      throw new Error("Menu id 'parts' is reserved and cannot be a menu");
    }

    if (!ID_SHAPE.test(menu.id)) {
      throw new Error(`Menu id '${menu.id}' does not match pattern [a-z][a-z0-9-]*`);
    }

    if (seenIds.has(menu.id)) {
      throw new Error(`Duplicate menu id '${menu.id}'`);
    }
    seenIds.add(menu.id);

    if (seenOrders.has(menu.order)) {
      throw new Error(`Duplicate menu order '${menu.order}'`);
    }
    seenOrders.add(menu.order);

    result.push(menu);
  }

  result.sort((a, b) => a.order - b.order);
  return result;
}

const rawMenus = import.meta.glob<{ menu: AdminMenu }>('./*/menu.ts', {
  eager: true,
});

export const menus: AdminMenu[] = collectMenus(rawMenus);
