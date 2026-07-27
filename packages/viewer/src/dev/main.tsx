/**
 * The development harness.
 *
 * Also the page the golden suite drives: `?fixture=…&slide=…&headless=1` renders one
 * slide at a fixed size and sets `window.__pptxReady`, which is what the Playwright
 * runner waits on before screenshotting.
 */

import { StrictMode, useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';

import { Presentation } from '../presentation.js';
import { PresentationViewer, type PresentationViewerHandle } from '../react.js';
import type { PresentationInfo } from '../types.js';

declare global {
  interface Window {
    __pptxReady?: boolean;
    __pptxError?: string;
    __pptxTrace?: string;
    /** Exposed for the benchmark, which drives the API layer directly. */
    __pptx?: { Presentation: typeof Presentation };
  }
}

// The bench measures the API layer without React in the way, so it needs a handle on it.
window.__pptx = { Presentation };

const params = new URLSearchParams(location.search);
const headless = params.get('headless') === '1';
const fixture = params.get('fixture');
const slideParam = Number.parseInt(params.get('slide') ?? '0', 10);
const renderWidth = Number.parseInt(params.get('w') ?? '960', 10);
const renderHeight = Number.parseInt(params.get('h') ?? '540', 10);

function Harness() {
  const [src, setSrc] = useState<string | File | null>(
    fixture ? `/fixtures/generated/${fixture}` : null,
  );
  const [info, setInfo] = useState<PresentationInfo | null>(null);
  const [slide, setSlide] = useState(Number.isFinite(slideParam) ? slideParam : 0);
  const [error, setError] = useState<string | null>(null);
  const viewer = useRef<PresentationViewerHandle | null>(null);

  const onLoad = useCallback((loaded: PresentationInfo) => {
    setInfo(loaded);
  }, []);

  // Signal readiness only once nothing is outstanding *and* the frame that drew it has
  // been painted. Waiting on a fixed delay instead would either screenshot a slide before
  // its images decoded or add that delay to every slide in the suite.
  useEffect(() => {
    if (!headless || !info) return;
    let cancelled = false;
    let raf = 0;
    let poll = 0;

    const settle = () => {
      if (cancelled) return;
      const deck = viewer.current?.presentation;
      if (deck && deck.pendingAssetCount() > 0) {
        poll = window.setTimeout(settle, 50);
        return;
      }
      // Two frames: one for the redraw that the assets triggered, one to be sure it has
      // been composited before the screenshot.
      raf = requestAnimationFrame(() => {
        raf = requestAnimationFrame(() => {
          if (cancelled) return;
          window.__pptxTrace = viewer.current?.presentation?.debugTrace(slide) ?? '';
          window.__pptxReady = true;
        });
      });
    };
    // One tick to let the first render kick decoding off before we start polling.
    poll = window.setTimeout(settle, 60);

    return () => {
      cancelled = true;
      window.clearTimeout(poll);
      cancelAnimationFrame(raf);
    };
  }, [info, slide]);

  useEffect(() => {
    if (headless && error) window.__pptxError = error;
  }, [error]);

  if (headless) {
    return (
      <div style={{ width: renderWidth, height: renderHeight, background: '#fff' }}>
        {src && (
          <PresentationViewer
            ref={viewer}
            src={src}
            slide={slide}
            width={renderWidth}
            height={renderHeight}
            fit="contain"
            keyboard={false}
            loading={null}
            onLoad={onLoad}
            onError={(e) => setError(e.message)}
          />
        )}
      </div>
    );
  }

  return (
    <div style={{ font: '14px system-ui, sans-serif', padding: 16 }}>
      <h1 style={{ fontSize: 18, margin: '0 0 12px' }}>pptx-viewer</h1>

      <div style={{ display: 'flex', gap: 12, alignItems: 'center', marginBottom: 12 }}>
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
        <button onClick={() => viewer.current?.previous()} disabled={!info || slide === 0}>
          ← Prev
        </button>
        <span>
          {info ? `${slide + 1} / ${info.slideCount}` : '—'}
        </span>
        <button
          onClick={() => viewer.current?.next()}
          disabled={!info || slide >= info.slideCount - 1}
        >
          Next →
        </button>
        {info && (
          <span style={{ color: '#666' }}>
            {info.width}×{info.height}pt
            {info.embeddedFonts.length > 0 &&
              ` · ${info.embeddedFonts.length} embedded font(s)`}
          </span>
        )}
      </div>

      <div
        style={{
          border: '1px solid #ddd',
          height: '70vh',
          background: '#f6f6f6',
        }}
      >
        {src ? (
          <PresentationViewer
            ref={viewer}
            src={src}
            slide={slide}
            width="100%"
            height="100%"
            onLoad={onLoad}
            onSlideChange={setSlide}
            onError={(e) => setError(e.message)}
          />
        ) : (
          <div style={{ display: 'grid', placeItems: 'center', height: '100%', color: '#888' }}>
            Choose a .pptx file, or run <code>npm run fixtures</code> and load one from{' '}
            <code>fixtures/generated/</code>.
          </div>
        )}
      </div>

      {error && <p style={{ color: '#b00020' }}>{error}</p>}

      {info && (
        <details style={{ marginTop: 12 }}>
          <summary style={{ cursor: 'pointer' }}>Diagnostics for this slide</summary>
          <p style={{ color: '#666', margin: '8px 0' }}>
            GPU backend would need: {viewer.current?.presentation?.gpuRequirements(slide)}
          </p>
          <pre
            style={{
              maxHeight: 320,
              overflow: 'auto',
              background: '#111',
              color: '#ddd',
              padding: 12,
              fontSize: 12,
            }}
          >
            {viewer.current?.presentation?.debugTrace(slide)}
          </pre>
        </details>
      )}
    </div>
  );
}

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    headless ? (
      <Harness />
    ) : (
      <StrictMode>
        <Harness />
      </StrictMode>
    ),
  );
}
