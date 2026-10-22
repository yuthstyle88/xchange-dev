import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { RequestQr } from './RequestQr';
import './request-qr.css';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(() => <RequestQr />, root);
}
