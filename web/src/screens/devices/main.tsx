import { Show } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { sharedSession } from '../../shared/session/session';
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
  const queryBranch = params.get('branch');
  const session = sharedSession();
  const branch = () => queryBranch || session.me()?.branch;
  const api = new ApiClient();

  render(
    () => (
      <Show when={branch()} keyed>
        {(b) => (
          <DevicesScreen
            api={api}
            branch={b}
            newId={() => crypto.randomUUID()}
            download={downloadFile}
          />
        )}
      </Show>
    ),
    appEl,
  );
}
