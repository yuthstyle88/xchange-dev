import { render } from 'solid-js/web';
import { StaffGuide } from './StaffGuide';

const root = document.getElementById('root');
if (root) {
  render(() => <StaffGuide />, root);
}
