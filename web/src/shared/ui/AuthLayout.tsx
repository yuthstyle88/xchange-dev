import type { JSX } from 'solid-js';
import { LangSwitch } from '../i18n/LangSwitch';
import { VersionFooter } from './VersionFooter';
import '@fontsource-variable/noto-sans-thai/wght.css';
import './xc-theme.css';
import './auth.css';

export function AuthLayout(props: {
  title: string;
  subtitle?: string;
  help?: JSX.Element;
  children?: JSX.Element;
}): JSX.Element {
  return (
    <div class="xc-auth xc-theme">
      <main class="xc-auth-card">
        <header class="xc-auth-header">
          <div class="xc-auth-header-top">
            <span class="xc-auth-brand">XCHANGE × T.K.</span>
            <LangSwitch />
          </div>
          <h1>{props.title}</h1>
          {props.subtitle && <p class="xc-auth-subtitle">{props.subtitle}</p>}
        </header>
        {props.help}
        {props.children}
      </main>
      <VersionFooter />
    </div>
  );
}
