import type { ComponentProps, ReactNode } from 'react';
import '../styles/controls.css';

type Props = ComponentProps<'button'> & {
  variant?: 'primary' | 'accent' | 'outline' | 'ghost';
  height?: 28 | 30 | 34 | 38;
  icon?: ReactNode;
};

export function Button({
  variant = 'primary',
  height = 30,
  icon,
  children,
  className = '',
  type = 'button',
  tabIndex = 0,
  style,
  ...props
}: Props) {
  return (
    <button
      {...props}
      type={type}
      tabIndex={tabIndex}
      className={`xt-button ${className}`}
      data-variant={variant}
      style={{ height, ...style }}
    >
      {icon && (
        <span aria-hidden="true" className="xt-control-icon">
          {icon}
        </span>
      )}
      {children}
    </button>
  );
}
