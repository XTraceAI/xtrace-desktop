import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeScope } from '../theme/ThemeProvider';
import { stories } from './stories';
import '../index.css';
import './gallery.css';

if (import.meta.env.DEV && import.meta.env.VITE_GALLERY === '1') {
  const story = stories.find(
    ({ id }) => id === decodeURIComponent(document.documentElement.dataset.story ?? ''),
  );
  const theme = document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
  if (story) {
    const { Render } = story;
    createRoot(document.getElementById('gallery-frame')!).render(
      <StrictMode>
        <ThemeScope theme={theme} className="gallery-frame" data-story-id={story.id}>
          <Render theme={theme} />
        </ThemeScope>
      </StrictMode>,
    );
  }
}
