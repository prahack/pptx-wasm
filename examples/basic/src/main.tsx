/**
 * The smallest useful pptx-wasm app.
 *
 * Deliberately uses nothing but the documented public API — if this file ever needs an
 * undocumented export or a deep import, the package's surface is wrong.
 */

import { useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';

import type { PresentationInfo } from 'pptx-wasm';
import { PresentationViewer, type PresentationViewerHandle } from 'pptx-wasm/react';

function App() {
  const viewer = useRef<PresentationViewerHandle>(null);
  const [src, setSrc] = useState<string | File>('/deck.pptx');
  const [info, setInfo] = useState<PresentationInfo | null>(null);
  const [slide, setSlide] = useState(0);
  const [error, setError] = useState<string | null>(null);

  return (
    <main style={{ font: '15px system-ui, sans-serif', padding: 24, maxWidth: 1200, margin: '0 auto' }}>
      <h1 style={{ fontSize: 20, marginTop: 0 }}>pptx-wasm</h1>

      <div style={{ display: 'flex', gap: 12, alignItems: 'center', marginBottom: 16, flexWrap: 'wrap' }}>
        <input
          type="file"
          accept=".pptx"
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) {
              setSrc(file);
              setSlide(0);
              setError(null);
            }
          }}
        />
        <button onClick={() => viewer.current?.previous()} disabled={slide === 0}>
          ‹ Previous
        </button>
        <span style={{ minWidth: 90, textAlign: 'center' }}>
          {info ? `${slide + 1} of ${info.slideCount}` : '—'}
        </span>
        <button
          onClick={() => viewer.current?.next()}
          disabled={!info || slide >= info.slideCount - 1}
        >
          Next ›
        </button>
      </div>

      <div style={{ border: '1px solid #ddd', borderRadius: 6, overflow: 'hidden', background: '#fafafa' }}>
        <PresentationViewer
          ref={viewer}
          src={src}
          height="70vh"
          onLoad={setInfo}
          onSlideChange={setSlide}
          onError={(e) => setError(e.message)}
        />
      </div>

      {error && (
        <p role="alert" style={{ color: '#b00020' }}>
          {error}
        </p>
      )}

      {info && viewer.current?.presentation?.notes(slide) && (
        <section style={{ marginTop: 16 }}>
          <h2 style={{ fontSize: 15 }}>Speaker notes</h2>
          <p style={{ whiteSpace: 'pre-wrap', color: '#444' }}>
            {viewer.current.presentation.notes(slide)}
          </p>
        </section>
      )}
    </main>
  );
}

const root = document.getElementById('root');
if (root) createRoot(root).render(<App />);
