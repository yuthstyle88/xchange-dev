import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { DevicesScreen } from './DevicesScreen';

function downloadFile(name: string, blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

const appEl = document.getElementById('app');
if (appEl) {
  const params = new URLSearchParams(window.location.search);
  const branch = params.get('branch') || 'MAIN';
  const api = new ApiClient();

  render(
    () => (
      <DevicesScreen
        api={api}
        branch={branch}
        newId={() => crypto.randomUUID()}
        download={downloadFile}
      />
    ),
    appEl,
  );
}
