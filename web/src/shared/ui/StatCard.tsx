import type { JSX } from 'solid-js';
import { formatMoney } from './MoneyText';
import './card.css';

export interface StatCardProps {
  label: string;
  value: string;
  note: string;
  unit?: string;
  format?: 'money' | 'plain';
}

export function StatCard(props: StatCardProps): JSX.Element {
  const formattedValue = () =>
    props.format === 'plain' ? props.value : formatMoney(props.value);

  return (
    <div class="stat-card">
      <div class="stat-label">{props.label}</div>
      <div class="stat-value">
        <span class="value">{formattedValue()}</span>{' '}
        <span class="unit">{props.unit ?? 'THB'}</span>
      </div>
      <div class="stat-note">{props.note}</div>
    </div>
  );
}
