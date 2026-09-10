import { useEffect } from 'react';
import { useNavigate } from 'react-router';
export function useShellShortcuts() {
  const navigate = useNavigate();
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.metaKey || event.altKey || event.ctrlKey || event.shiftKey || event.isComposing)
        return;
      if (event.key === ',') {
        event.preventDefault();
        void navigate('/settings');
      }
      if (event.key.toLowerCase() === 'k') {
        const search = document.querySelector<HTMLInputElement>(
          'main input[type="search"]:not([disabled])',
        );
        if (search) {
          event.preventDefault();
          search.focus();
        }
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [navigate]);
}
