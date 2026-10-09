import type { JSX } from 'solid-js';
import { LangSwitch } from '../i18n/LangSwitch';
import { VersionFooter } from './VersionFooter';
import './auth.css';

export function AuthLayout(props: {
  title: string;
  help?: JSX.Element;
  children?: JSX.Element;
}): JSX.Element {
  return (
    <div class="xc-auth">
      <main class="xc-auth-card">
        <header class="xc-auth-header">
          <div class="xc-auth-header-top">
            <span class="xc-auth-brand">XCHANGE × T.K.</span>
            <LangSwitch />
          </div>
          <h1>{props.title}</h1>
        </header>
        {props.help}
        {props.children}
      </main>
      <VersionFooter />
    </div>
  );
}
