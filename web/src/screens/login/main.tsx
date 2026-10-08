import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { authApi } from './auth-api';
import { LoginScreen } from './LoginScreen';

const root = document.getElementById('root');
if (root) {
  const client = new ApiClient();
  const auth = authApi(client);
  render(
    () => <LoginScreen auth={auth} navigate={(url) => window.location.assign(url)} />,
    root,
  );
}
