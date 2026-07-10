type StatusBadgeProps = {
  children: string;
  tone?: 'accent' | 'success';
};

export function StatusBadge({ children, tone = 'accent' }: StatusBadgeProps) {
  return (
    <span className={`status-badge status-badge--${tone}`}>
      <span className="status-badge__dot" aria-hidden="true" />
      {children}
    </span>
  );
}
