import { render } from 'solid-js/web';
import { ReceiptSample } from './ReceiptSample';

const root = document.getElementById('root');
if (root) {
  render(() => <ReceiptSample />, root);
}
