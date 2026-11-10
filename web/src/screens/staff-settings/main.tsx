import { render } from 'solid-js/web';
import { StaffSettings } from './StaffSettings';
import './staff-settings.css';

const root = document.getElementById('app');
if (root) {
  render(() => <StaffSettings />, root);
}
