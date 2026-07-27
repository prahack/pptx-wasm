/**
 * Side-by-side comparison of pptx renderers.
 *
 * Two jobs. Interactively it puts this viewer next to `pptx-preview` on the same file so
 * differences are visible rather than argued about. Headlessly
 * (`?headless=1&engine=…&fixture=…`) it renders exactly one engine and publishes its
 * timings, which is what `tests/golden/compare.mjs` drives.
 *
 * Fairness rules, since a benchmark that flatters its author is worthless:
 *  - both engines get the same file, the same slide and the same pixel dimensions;
 *  - timing starts before the fetch and stops when the engine says it has drawn;
 *  - each engine is measured in its own page load, so neither warms the other's caches.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';

import { Presentation } from 'pptx-viewer';
import { init as initPptxPreview } from 'pptx-preview';

export type EngineId = 'ours' | 'pptx-preview';

export interface Timing {
  /** Fetching the bytes. Reported separately so it can be excluded from the comparison. */
  fetchMs: number;
  /** Parsing/opening the deck. */
  openMs: number;
  /** Producing the first visible slide. */
  renderMs: number;
  /** open + render, which is what a user waits for. */
  totalMs: number;
  slideCount: number;
}

declare global {
  interface Window {
    __cmpReady?: boolean;
    __cmpError?: string;
    __cmpTiming?: Timing;
    /**
     * Runs one more open-and-render in this page and returns its timings.
     *
     * The harness calls this repeatedly to separate the *cold* cost — which for a
     * WASM engine includes instantiating the module, and for a JS engine includes
     * parsing and JIT-warming the bundle — from the *warm* cost of opening a second
     * deck in a session. Both are real; they answer different questions.
     */
    __cmpRun?: () => Promise<Timing>;
  }
}

const params = new URLSearchParams(location.search);
const headless = params.get('headless') === '1';
const engineParam = (params.get('engine') as EngineId | null) ?? 'ours';
const fixtureParam = params.get('fixture') ?? 'm1-basic.pptx';
const slideParam = Number.parseInt(params.get('slide') ?? '0', 10) || 0;
const W = Number.parseInt(params.get('w') ?? '960', 10);
const H = Number.parseInt(params.get('h') ?? '540', 10);

const FIXTURES = [
  'm0-blank.pptx',
  'm1-basic.pptx',
  'm2-text.pptx',
  'm3-shapes.pptx',
  'm4-template.pptx',
  'm5a-tables.pptx',
  'm5b-charts.pptx',
  'm5c-effects.pptx',
];

async function fetchDeck(name: string): Promise<{ bytes: ArrayBuffer; fetchMs: number }> {
  const t = performance.now();
  const res = await fetch(`/fixtures/generated/${name}`);
  if (!res.ok) throw new Error(`could not fetch ${name}: ${res.status}`);
  const bytes = await res.arrayBuffer();
  return { bytes, fetchMs: performance.now() - t };
}

/** Renders with this project's viewer. */
async function renderOurs(
  host: HTMLDivElement,
  bytes: ArrayBuffer,
  slide: number,
): Promise<Timing> {
  const { fetchMs } = { fetchMs: 0 };
  host.replaceChildren();
  const canvas = document.createElement('canvas');
  canvas.style.width = `${W}px`;
  canvas.style.height = `${H}px`;
  host.appendChild(canvas);

  const t0 = performance.now();
  const deck = await Presentation.open(bytes.slice(0));
  const openMs = performance.now() - t0;

  const t1 = performance.now();
  await deck.render(slide, canvas, { width: W, height: H, dpr: 1, fit: 'contain' });
  // Images decode off the render path; wait for them so the comparison is of a finished
  // frame, not a partial one.
  if (deck.pendingAssetCount() > 0) {
    await new Promise<void>((resolve) => {
      const stop = deck.onAssetsReady(() => {
        stop();
        resolve();
      });
      setTimeout(() => {
        stop();
        resolve();
      }, 5000);
    });
    await deck.render(slide, canvas, { width: W, height: H, dpr: 1, fit: 'contain' });
  }
  const renderMs = performance.now() - t1;

  return {
    fetchMs,
    openMs,
    renderMs,
    totalMs: openMs + renderMs,
    slideCount: deck.slideCount,
  };
}

/** Renders with pptx-preview, which draws HTML into a host element. */
async function renderPptxPreview(
  host: HTMLDivElement,
  bytes: ArrayBuffer,
  slide: number,
): Promise<Timing> {
  host.replaceChildren();
  const mount = document.createElement('div');
  mount.style.width = `${W}px`;
  mount.style.height = `${H}px`;
  host.appendChild(mount);

  const previewer = initPptxPreview(mount, { width: W, height: H, mode: 'slide' });

  const t0 = performance.now();
  await previewer.load(bytes.slice(0));
  const openMs = performance.now() - t0;

  const t1 = performance.now();
  previewer.renderSingleSlide(slide);
  // Its render is synchronous, but let the browser lay the resulting DOM out before
  // calling it done — otherwise we would be timing less work than the user waits for.
  await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
  const renderMs = performance.now() - t1;

  return {
    fetchMs: 0,
    openMs,
    renderMs,
    totalMs: openMs + renderMs,
    slideCount: previewer.slideCount ?? 1,
  };
}

const ENGINES: Record<EngineId, { label: string; run: typeof renderOurs }> = {
  ours: { label: 'pptx-viewer (this project)', run: renderOurs },
  'pptx-preview': { label: 'pptx-preview 1.0.7', run: renderPptxPreview },
};

function Pane({
  engine,
  fixture,
  slide,
  onTiming,
}: {
  engine: EngineId;
  fixture: string;
  slide: number;
  onTiming?: (t: Timing) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const [timing, setTiming] = useState<Timing | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const el = host.current;
    if (!el) return;
    setTiming(null);
    setError(null);

    (async () => {
      const { bytes, fetchMs } = await fetchDeck(fixture);
      if (cancelled) return;
      const result = await ENGINES[engine].run(el, bytes, slide);
      if (cancelled) return;
      const withFetch = { ...result, fetchMs };
      setTiming(withFetch);
      onTiming?.(withFetch);
    })().catch((e: unknown) => {
      if (cancelled) return;
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      if (headless) window.__cmpError = message;
    });

    return () => {
      cancelled = true;
    };
  }, [engine, fixture, slide, onTiming]);

  return (
    <section style={{ flex: '0 0 auto' }}>
      {!headless && (
        <header style={{ marginBottom: 6 }}>
          <strong>{ENGINES[engine].label}</strong>
          <div style={{ color: '#666', fontSize: 13, minHeight: 18 }}>
            {error
              ? `error: ${error}`
              : timing
                ? `open ${timing.openMs.toFixed(1)}ms · render ${timing.renderMs.toFixed(1)}ms · total ${timing.totalMs.toFixed(1)}ms`
                : 'rendering…'}
          </div>
        </header>
      )}
      <div
        ref={host}
        data-engine={engine}
        style={{
          width: W,
          height: H,
          background: '#fff',
          border: headless ? 'none' : '1px solid #ddd',
          overflow: 'hidden',
          position: 'relative',
        }}
      />
    </section>
  );
}

function Headless() {
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = host.current;
    if (!el) return;

    const runOnce = async (): Promise<Timing> => {
      const { bytes, fetchMs } = await fetchDeck(fixtureParam);
      const result = await ENGINES[engineParam].run(el, bytes, slideParam);
      return { ...result, fetchMs };
    };

    window.__cmpRun = runOnce;

    runOnce()
      .then((t) => {
        window.__cmpTiming = t;
        // Two frames so a screenshot cannot catch a half-composited slide.
        requestAnimationFrame(() =>
          requestAnimationFrame(() => {
            window.__cmpReady = true;
          }),
        );
      })
      .catch((e: unknown) => {
        window.__cmpError = e instanceof Error ? e.message : String(e);
        window.__cmpReady = true;
      });
  }, []);

  return (
    <div style={{ width: W, height: H, background: '#fff' }}>
      <div
        ref={host}
        data-engine={engineParam}
        style={{ width: W, height: H, background: '#fff', overflow: 'hidden', position: 'relative' }}
      />
    </div>
  );
}

function App() {
  const [fixture, setFixture] = useState(fixtureParam);
  const [slide, setSlide] = useState(slideParam);
  const [nonce, setNonce] = useState(0);

  return (
    <main style={{ font: '14px system-ui, sans-serif', padding: 20 }}>
      <h1 style={{ fontSize: 19, marginTop: 0 }}>pptx renderer comparison</h1>
      <p style={{ color: '#555', maxWidth: 760, marginTop: 0 }}>
        Same file, same slide, same pixel size, rendered by both engines. Timings here are
        indicative — they share a page and a warm cache. For the measured numbers run{' '}
        <code>npm run compare</code>, which loads each engine in its own page.
      </p>

      <div style={{ display: 'flex', gap: 12, alignItems: 'center', marginBottom: 16 }}>
        <label>
          Fixture{' '}
          <select
            value={fixture}
            onChange={(e) => {
              setFixture(e.target.value);
              setSlide(0);
            }}
          >
            {FIXTURES.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
          </select>
        </label>
        <label>
          Slide{' '}
          <input
            type="number"
            min={0}
            value={slide}
            style={{ width: 60 }}
            onChange={(e) => setSlide(Math.max(0, Number.parseInt(e.target.value, 10) || 0))}
          />
        </label>
        <button onClick={() => setNonce((n) => n + 1)}>Re-render</button>
      </div>

      <div style={{ display: 'flex', gap: 20, flexWrap: 'wrap' }}>
        <Pane key={`ours-${nonce}`} engine="ours" fixture={fixture} slide={slide} />
        <Pane key={`prev-${nonce}`} engine="pptx-preview" fixture={fixture} slide={slide} />
      </div>
    </main>
  );
}

const root = document.getElementById('root');
if (root) createRoot(root).render(headless ? <Headless /> : <App />);
