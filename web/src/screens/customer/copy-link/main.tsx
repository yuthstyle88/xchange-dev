import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CopyLink } from './CopyLink';
import './copy-link.css';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(() => <CopyLink fetch={(url, init) => fetch(url, init)} />, root);
}
