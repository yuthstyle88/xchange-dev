import type { JSX } from 'solid-js';
import { Show } from 'solid-js';
import './card.css';

export interface CardProps {
  tone?: 'plain' | 'dark';
  class?: string;
  children?: JSX.Element;
}

export function Card(props: CardProps): JSX.Element {
  const cardClass = () => {
    const base = props.tone === 'dark' ? 'card dark-card' : 'card';
    return props.class ? `${base} ${props.class}` : base;
  };
  return <section class={cardClass()}>{props.children}</section>;
}

export interface CardHeadProps {
  eyebrow?: string;
  title: string;
  level?: 2 | 3;
  aside?: JSX.Element;
  class?: string;
}

export function CardHead(props: CardHeadProps): JSX.Element {
  const headingClass = () =>
    props.class ? `card-heading ${props.class}` : 'card-heading';
  return (
    <div class={headingClass()}>
      <div>
        <Show when={props.eyebrow}>
          <p class="eyebrow">{props.eyebrow}</p>
        </Show>
        <Show
          when={props.level === 3}
          fallback={<h2 class="card-title">{props.title}</h2>}
        >
          <h3 class="card-title">{props.title}</h3>
        </Show>
      </div>
      {props.aside}
    </div>
  );
}

export interface CardBodyProps {
  flush?: boolean;
  class?: string;
  children?: JSX.Element;
}

export function CardBody(props: CardBodyProps): JSX.Element {
  const bodyClass = () => {
    const base = props.flush ? 'card-body flush' : 'card-body';
    return props.class ? `${base} ${props.class}` : base;
  };
  return <div class={bodyClass()}>{props.children}</div>;
}

export interface CardActionsProps {
  class?: string;
  children?: JSX.Element;
}

export function CardActions(props: CardActionsProps): JSX.Element {
  const actionsClass = () =>
    props.class ? `card-actions ${props.class}` : 'card-actions';
  return <div class={actionsClass()}>{props.children}</div>;
}

export interface CardGridProps {
  columns?: 2 | 3 | 4;
  class?: string;
  children?: JSX.Element;
}

export function CardGrid(props: CardGridProps): JSX.Element {
  const gridClass = () => {
    const cols = props.columns ?? 4;
    const base = `card-grid cols-${cols}`;
    return props.class ? `${base} ${props.class}` : base;
  };
  return <div class={gridClass()}>{props.children}</div>;
}
