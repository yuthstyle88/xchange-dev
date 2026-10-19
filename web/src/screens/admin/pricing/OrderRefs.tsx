// §9 OrderRefs: recent orders with the market they were priced against (FE pricing-board skeleton stub, #564; docs/design/blueprint/pricing-tiers.md §9). Filled by its own ticket.
import type { JSX } from 'solid-js';
import type { OrderRefView } from './pricing-board.types';
import './order-refs.css';

export interface OrderRefsProps {
  refs: readonly OrderRefView[];
}

export function OrderRefs(_props: OrderRefsProps): JSX.Element {
  return null;
}
