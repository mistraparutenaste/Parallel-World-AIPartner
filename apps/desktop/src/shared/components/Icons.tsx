import type { ReactNode, SVGProps } from 'react';

type IconName = 'chat' | 'database' | 'diagnostic' | 'microphone' | 'model' | 'speaker' | 'user';

type IconProps = SVGProps<SVGSVGElement> & { name: IconName };

const paths: Record<IconName, ReactNode> = {
  chat: <path d="M4 5.5A2.5 2.5 0 0 1 6.5 3h7A2.5 2.5 0 0 1 16 5.5v5a2.5 2.5 0 0 1-2.5 2.5H9l-4 3v-3.5A2.5 2.5 0 0 1 4 10.5z" />,
  database: <><ellipse cx="10" cy="5" rx="6" ry="2.5" /><path d="M4 5v5c0 1.4 2.7 2.5 6 2.5s6-1.1 6-2.5V5M4 10v5c0 1.4 2.7 2.5 6 2.5s6-1.1 6-2.5v-5" /></>,
  diagnostic: <path d="M2 12h4l2-7 4 12 2-7h4" />,
  microphone: <><rect x="7" y="2" width="6" height="11" rx="3" /><path d="M4.5 10a5.5 5.5 0 0 0 11 0M10 15.5V19M7 19h6" /></>,
  model: <path d="M8 2.5a3 3 0 0 0-3 3v1a3 3 0 0 0 0 5v1a3 3 0 0 0 3 3.5M12 2.5a3 3 0 0 1 3 3v1a3 3 0 0 1 0 5v1a3 3 0 0 1-3 3.5M10 2v16M6 7h2M12 7h2M6 13h2M12 13h2" />,
  speaker: <><path d="M3 8h4l4-4v12l-4-4H3z" /><path d="M14 7a4 4 0 0 1 0 6M16.5 4.5a7.5 7.5 0 0 1 0 11" /></>,
  user: <><circle cx="10" cy="6" r="3" /><path d="M4 18v-2a6 6 0 0 1 12 0v2z" /></>,
};

export function Icon({ name, ...props }: IconProps) {
  return (
    <svg viewBox="0 0 20 20" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...props}>
      {paths[name]}
    </svg>
  );
}
