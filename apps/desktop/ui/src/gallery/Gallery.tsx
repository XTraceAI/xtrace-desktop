import { useSearchParams } from 'react-router';
import { stories } from './stories';
import { sample } from './sample';
import './gallery.css';

export default function Gallery() {
  const [params, setParams] = useSearchParams();
  const selected = stories.find(({ id }) => id === params.get('story')) ?? stories[0];
  return (
    <div className="gallery">
      <aside className="gallery-index" aria-label="Story index">
        <h1>Component gallery</h1>
        <p>{sample.label}</p>
        <p>{stories.length} stories · paired themes</p>
        <nav>
          {stories.map(({ id }) => (
            <button
              key={id}
              type="button"
              tabIndex={0}
              aria-current={selected.id === id ? 'page' : undefined}
              onClick={() => setParams({ story: id })}
            >
              {id}
            </button>
          ))}
        </nav>
      </aside>
      <main className="gallery-main">
        <header>
          <h2>{selected.id}</h2>
          <p>
            {selected.size[0]} × {selected.size[1]} CSS pixels · scroll to inspect full-size frames
          </p>
          <p>Illustrative states; no independent design comparison.</p>
        </header>
        <div className="gallery-pair">
          {(['dark', 'light'] as const).map((theme) => (
            <section key={theme}>
              <h3>{theme}</h3>
              <iframe
                key={`${selected.id}-${theme}`}
                title={`${selected.id} · ${theme}`}
                width={selected.size[0]}
                height={selected.size[1]}
                srcDoc={`<!doctype html><html data-story="${encodeURIComponent(selected.id)}" data-theme="${theme}"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="gallery-frame"></div><script type="module" src="/src/gallery/frame.tsx"></script></body></html>`}
              />
            </section>
          ))}
        </div>
      </main>
    </div>
  );
}
