import type { ReactNode } from 'react';

type WindowFrameProps = {
  children: ReactNode;
  title: string;
  status?: ReactNode;
};

export function WindowFrame({ children, status, title }: WindowFrameProps) {
  return (
    <main className="window-frame">
      <header className="window-frame__titlebar">
        <span className="window-frame__drag" aria-hidden="true">
          {Array.from({ length: 9 }, (_, index) => <i key={index} />)}
        </span>
        <div className="window-frame__title">{title}</div>
        {status ? <div className="window-frame__status">{status}</div> : null}
      </header>
      {children}
    </main>
  );
}
