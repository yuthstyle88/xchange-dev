// V2-parts (#476): stat card (ticket §7, design §12.2, P1, P6)

import type { JSX } from 'solid-js';
import { formatMoney } from '../../../shared/ui/MoneyText';
import './parts.css';

export interface StatCardProps {
  label: string;
  value: string;
  note: string;
  unit?: string;
}

export function StatCard(props: StatCardProps): JSX.Element {
  return (
    <div class="stat-card">
      <div class="stat-label">{props.label}</div>
      <div class="stat-value">
        <span class="value">{formatMoney(props.value)}</span>{' '}
        <span class="unit">{props.unit ?? 'THB'}</span>
      </div>
      <div class="stat-note">{props.note}</div>
    </div>
  );
}
