import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import './queue-display.css';
import { QueueDisplay } from './QueueDisplay';

setAudience('customer');

const root = document.getElementById('root');
if (!root) throw new Error('Missing #root element');

render(
  () => <QueueDisplay fetch={(url, init) => fetch(url, init)} search={window.location.search} />,
  root,
);
