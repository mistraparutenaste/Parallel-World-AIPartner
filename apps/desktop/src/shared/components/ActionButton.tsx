import type { ButtonHTMLAttributes, ReactNode } from 'react';

type ActionButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  children: ReactNode;
  variant?: 'primary' | 'secondary';
};

export function ActionButton({ children, className = '', variant = 'secondary', ...props }: ActionButtonProps) {
  return (
    <button className={`action-button action-button--${variant} ${className}`.trim()} {...props}>
      {children}
    </button>
  );
}
