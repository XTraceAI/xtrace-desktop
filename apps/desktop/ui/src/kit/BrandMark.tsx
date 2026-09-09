import '../styles/sidebar.css';

export interface BrandMarkProps {
  size?: 20 | 22 | 26 | 34;
  showLabel?: boolean;
}

export function BrandMark({ size = 26, showLabel = true }: BrandMarkProps) {
  return (
    <span className="xt-brand-mark">
      <img src="/sidebar-mark.png" width={size} height={size} alt={showLabel ? '' : 'XTrace'} />
      {showLabel && <span>XTrace</span>}
    </span>
  );
}
