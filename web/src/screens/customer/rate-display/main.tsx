import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { RateDisplay } from './RateDisplay';
import './rate-display.css';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <RateDisplay
        fetch={(url, init) => fetch(url, init)}
        search={window.location.search}
        ratesIntervalMs={60_000}
        branchesIntervalMs={600_000}
      />
    ),
    root,
  );
}
