/**
 * Side-by-side comparison of pptx renderers.
 *
 * Two jobs. Interactively it puts this viewer next to every other engine on the same file —
 * a bundled fixture or one you drop in — so differences are visible rather than argued
 * about. Headlessly (`?headless=1&engine=…&fixture=…`) it renders exactly one engine and
 * publishes its timings, which is what `tests/golden/compare.mjs` drives.
 *
 * Fairness rules, since a benchmark that flatters its author is worthless:
 *  - every engine gets the same bytes, the same slide and the same pixel dimensions;
 *  - the clock stops when the engine says it has drawn, not when its promise resolves;
 *  - in the harness each engine is measured in its own page load, so neither warms the
 *    other's caches;
 *  - no engine is in the entry module graph, so each pays its own module and WASM
 *    instantiation inside its own cold measurement rather than before the clock;
 *  - an engine that fails is reported as failing, not silently dropped.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';

// Every engine — this one included — is imported dynamically from *inside* the timed
// region, and none is in the entry module graph. Two reasons.
//
// Fairness: "cold" is meant to be what a first-time visitor waits for, which includes
// whatever the engine must do once — instantiate a WASM module, or parse and JIT a JS
// bundle. Pre-resolving the imports moved the competitors' share of that outside the
// clock while this project's `initWasm` stayed inside it, and the resulting cold figure
// flattered us by several milliseconds. A static import here would do the same thing more
// quietly, which is why there is not one.
//
// Isolation: a package that fails to resolve breaks only its own measurement instead of
// taking the page down with it. pptxviewjs imports an undeclared `chart.js` and did
// exactly that, hiding every other engine's result behind its failure.

export type EngineId =
  | 'ours'
  | 'pptx-preview'
  | 'pptxviewjs'
  | 'aiden0z'
  | 'jvmr'
  | 'glimpse'
  | 'vanilla';

export interface Timing {
  /** Fetching or reading the bytes. Reported separately so it can be excluded. */
  fetchMs: number;
  /** Parsing/opening the deck. */
  openMs: number;
  /** Producing the first visible slide. */
  renderMs: number;
  /** open + render, which is what a user actually waits for. */
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
     * The harness calls this repeatedly for two reasons: to separate the *cold* cost —
     * which for a WASM engine includes instantiating the module, and for a JS engine
     * includes parsing and JIT-warming the bundle — from the *warm* cost of opening a
     * second deck; and to walk every slide, since judging a renderer on slide 1 alone
     * flatters it (slide 1 is usually the title, the simplest slide in the deck).
     */
    __cmpRun?: (slide?: number) => Promise<Timing>;
  }
}

const params = new URLSearchParams(location.search);
const headless = params.get('headless') === '1';
const engineParam = (params.get('engine') as EngineId | null) ?? 'ours';
const fixtureParam = params.get('fixture') ?? 'm1-basic.pptx';
const slideParam = Number.parseInt(params.get('slide') ?? '0', 10) || 0;
const W = Number.parseInt(params.get('w') ?? '960', 10);
const H = Number.parseInt(params.get('h') ?? '540', 10);

/**
 * Device pixel ratio to render at.
 *
 * Interactively this must be the display's real ratio, or a canvas renderer looks soft
 * against a DOM one on any Retina screen — the DOM engine gets native-resolution text for
 * free while the canvas is drawn at CSS resolution and upscaled. That is a difference in
 * the harness, not in the renderers, and it is exactly the kind of unfairness that makes
 * a benchmark worthless.
 *
 * Headlessly it is pinned to 1, because those screenshots are diffed against a 960x540
 * LibreOffice render and have to come out the same size.
 */
const DPR = headless ? 1 : (globalThis.devicePixelRatio || 1);

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

async function fetchFixture(name: string): Promise<{ bytes: ArrayBuffer; fetchMs: number }> {
  const t = performance.now();
  const res = await fetch(`/fixtures/generated/${name}`);
  if (!res.ok) throw new Error(`could not fetch ${name}: ${res.status}`);
  const bytes = await res.arrayBuffer();
  return { bytes, fetchMs: performance.now() - t };
}

/**
 * Forces style and layout for a DOM-based engine, and returns once they are done.
 *
 * A DOM renderer has not finished when its call returns — it has queued a tree the
 * browser has yet to lay out, and that cost is real. The obvious way to capture it is to
 * wait for a couple of animation frames, and that is what this harness did at first. It
 * was wrong: `requestAnimationFrame` is paced to vsync, so the wait cost a flat ~16.6ms
 * in headless Chromium no matter how much work the engine had done. Every DOM engine
 * scored an identical ~14.5ms on every fixture — the number was measuring the wait, not
 * the renderer, and it flattered this project's canvas path, which pays no such wait.
 *
 * Reading `offsetHeight` instead forces a synchronous style-and-layout pass and returns
 * immediately after it. Paint and composite are still deferred — but they are equally
 * deferred for a canvas, so the two are finally being timed to the same point.
 */
function flushLayout(el: HTMLElement): void {
  void el.offsetHeight;
  void el.getBoundingClientRect().height;
}

/** Renders with this project's viewer. */
async function renderOurs(host: HTMLElement, bytes: ArrayBuffer, slide: number): Promise<Timing> {
  host.replaceChildren();
  const canvas = document.createElement('canvas');
  canvas.style.width = `${W}px`;
  canvas.style.height = `${H}px`;
  host.appendChild(canvas);

  const t0 = performance.now();
  const { Presentation } = await import('pptx-wasm');
  // Each engine gets its own copy: a consumer is free to detach the buffer it is given.
  const deck = await Presentation.open(bytes.slice(0));
  const openMs = performance.now() - t0;

  const index = Math.min(Math.max(0, slide), Math.max(0, deck.slideCount - 1));
  const t1 = performance.now();
  await deck.render(index, canvas, { width: W, height: H, dpr: DPR, fit: 'contain' });
  // Images decode off the render path; wait for them so this is a finished frame rather
  // than a partial one.
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
    await deck.render(index, canvas, { width: W, height: H, dpr: DPR, fit: 'contain' });
  }
  const renderMs = performance.now() - t1;

  return { fetchMs: 0, openMs, renderMs, totalMs: openMs + renderMs, slideCount: deck.slideCount };
}

/** Renders with pptx-preview, which draws HTML into a host element. */
async function renderPptxPreview(
  host: HTMLElement,
  bytes: ArrayBuffer,
  slide: number,
): Promise<Timing> {
  host.replaceChildren();
  const mount = document.createElement('div');
  mount.style.width = `${W}px`;
  mount.style.height = `${H}px`;
  host.appendChild(mount);

  const t0 = performance.now();
  const { init: initPptxPreview } = await import('pptx-preview');
  const previewer = initPptxPreview(mount, { width: W, height: H, mode: 'slide' });
  await previewer.load(bytes.slice(0));
  const openMs = performance.now() - t0;

  const count = previewer.slideCount ?? 1;
  const index = Math.min(Math.max(0, slide), Math.max(0, count - 1));
  const t1 = performance.now();
  previewer.renderSingleSlide(index);
  flushLayout(mount);
  const renderMs = performance.now() - t1;

  return { fetchMs: 0, openMs, renderMs, totalMs: openMs + renderMs, slideCount: count };
}

/** Renders with pptxviewjs, which draws to a canvas we supply. */
async function renderPptxViewJs(
  host: HTMLElement,
  bytes: ArrayBuffer,
  slide: number,
): Promise<Timing> {
  host.replaceChildren();
  const canvas = document.createElement('canvas');
  canvas.style.width = `${W}px`;
  canvas.style.height = `${H}px`;
  host.appendChild(canvas);

  const t0 = performance.now();
  const { PPTXViewer } = await import('pptxviewjs');
  const viewer = new PPTXViewer({ canvas, slideSizeMode: 'fit' });
  await viewer.loadFile(bytes.slice(0));
  const openMs = performance.now() - t0;

  const count = viewer.getSlideCount?.() ?? 1;
  const index = Math.min(Math.max(0, slide), Math.max(0, count - 1));
  const t1 = performance.now();
  await viewer.renderSlide(index, canvas, { scale: DPR });
  const renderMs = performance.now() - t1;

  return { fetchMs: 0, openMs, renderMs, totalMs: openMs + renderMs, slideCount: count };
}

/** Renders with @aiden0z/pptx-renderer, which builds a DOM tree in a container. */
async function renderAiden(host: HTMLElement, bytes: ArrayBuffer, slide: number): Promise<Timing> {
  host.replaceChildren();
  const mount = document.createElement('div');
  mount.style.width = `${W}px`;
  mount.style.height = `${H}px`;
  host.appendChild(mount);

  const t0 = performance.now();
  const { PptxViewer: AidenViewer } = await import('@aiden0z/pptx-renderer');
  const viewer = new AidenViewer(mount, { width: W, fitMode: 'contain' });
  await viewer.open(bytes.slice(0), { renderMode: 'slide' });
  const openMs = performance.now() - t0;

  const count = viewer.slideCount || 1;
  const index = Math.min(Math.max(0, slide), Math.max(0, count - 1));
  const t1 = performance.now();
  await viewer.renderSlide(index);
  flushLayout(mount);
  const renderMs = performance.now() - t1;

  return { fetchMs: 0, openMs, renderMs, totalMs: openMs + renderMs, slideCount: count };
}

/**
 * Renders with @jvmr/pptx-to-html, which converts a deck to an array of HTML strings.
 *
 * Its whole API is one call that returns every slide at once, so the deck is fully parsed
 * during "open" however few slides are wanted. That is a real architectural difference
 * rather than a slow implementation: on these one-to-three-slide fixtures it costs almost
 * nothing, and on a 250-slide deck it is the difference between parsing one slide and
 * parsing all of them. The cache below keeps it from re-parsing on every warm run, which
 * is the fairest reading — a consumer would hold the returned array too.
 */
const jvmrCache = new WeakMap<ArrayBuffer, string[]>();

async function renderJvmr(host: HTMLElement, bytes: ArrayBuffer, slide: number): Promise<Timing> {
  host.replaceChildren();
  const mount = document.createElement('div');
  mount.style.width = `${W}px`;
  mount.style.height = `${H}px`;
  host.appendChild(mount);

  const t0 = performance.now();
  const { pptxToHtml } = await import('@jvmr/pptx-to-html');
  let slides = jvmrCache.get(bytes);
  if (!slides) {
    slides = await pptxToHtml(bytes.slice(0), { width: W, height: H });
    jvmrCache.set(bytes, slides);
  }
  const openMs = performance.now() - t0;

  const count = slides.length || 1;
  const index = Math.min(Math.max(0, slide), Math.max(0, count - 1));
  const t1 = performance.now();
  mount.innerHTML = slides[index] ?? '';
  flushLayout(mount);
  const renderMs = performance.now() - t1;

  return { fetchMs: 0, openMs, renderMs, totalMs: openMs + renderMs, slideCount: count };
}

/**
 * Renders with pptx-glimpse, which emits SVG.
 *
 * Uses its documented parse-once path — `readPptx` for the model, then one
 * `renderPptxSourceModelToSvg` per slide — rather than converting the whole deck each
 * time. Its API supports both, and picking the slower one would be misrepresenting it.
 */
const glimpseCache = new WeakMap<
  ArrayBuffer,
  { model: Awaited<ReturnType<typeof import('@pptx-glimpse/document').readPptx>>; count: number }
>();

async function renderGlimpse(host: HTMLElement, bytes: ArrayBuffer, slide: number): Promise<Timing> {
  host.replaceChildren();
  const mount = document.createElement('div');
  mount.style.width = `${W}px`;
  mount.style.height = `${H}px`;
  host.appendChild(mount);

  const t0 = performance.now();
  const [{ renderPptxSourceModelToSvg }, { readPptx }] = await Promise.all([
    import('pptx-glimpse'),
    import('@pptx-glimpse/document'),
  ]);
  let entry = glimpseCache.get(bytes);
  if (!entry) {
    const model = await readPptx(new Uint8Array(bytes.slice(0)));
    entry = { model, count: model.slides.length || 1 };
    glimpseCache.set(bytes, entry);
  }
  const openMs = performance.now() - t0;

  const index = Math.min(Math.max(0, slide), Math.max(0, entry.count - 1));
  const t1 = performance.now();
  const report = await renderPptxSourceModelToSvg(entry.model, {
    slides: [index + 1],
    width: W,
    height: H,
  });
  mount.innerHTML = report.slides[0]?.svg ?? '';
  // The SVG comes back at the slide's native size (1280x720 for a 16:9 deck), so it must
  // be fitted to the pane or it renders a third too large and clipped. Its viewBox does
  // the scaling; only the element's own width/height need overriding. Getting this wrong
  // would have scored the engine's geometry as broken when it is not.
  const svg = mount.querySelector('svg');
  if (svg) {
    svg.setAttribute('width', String(W));
    svg.setAttribute('height', String(H));
    svg.style.width = `${W}px`;
    svg.style.height = `${H}px`;
  }
  flushLayout(mount);
  const renderMs = performance.now() - t1;

  return { fetchMs: 0, openMs, renderMs, totalMs: openMs + renderMs, slideCount: entry.count };
}

/**
 * Renders with pptx-vanilla-viewer (the ChristopherVR engine's framework-free build).
 *
 * Its chrome — toolbar, thumbnail rail, inspector — is switched off. That is not to
 * flatter it: the accuracy score diffs the whole pane against a slide render, so leaving
 * a toolbar in frame would count its own UI as error and make the number meaningless.
 * What is being compared is the slide, for every engine.
 */
async function renderVanilla(host: HTMLElement, bytes: ArrayBuffer, slide: number): Promise<Timing> {
  host.replaceChildren();
  const mount = document.createElement('div');
  mount.style.width = `${W}px`;
  mount.style.height = `${H}px`;
  host.appendChild(mount);

  const t0 = performance.now();
  const mod = await import('pptx-vanilla-viewer');
  if (!document.getElementById('vanilla-css')) {
    const style = document.createElement('style');
    style.id = 'vanilla-css';
    style.textContent = mod.getViewerCss();
    document.head.appendChild(style);
  }
  const viewer = mod.createPptxViewer(mount, {
    readOnly: true,
    editable: false,
    showToolbar: false,
    showThumbnails: false,
    showInspector: false,
    showFormatToolbar: false,
  } as never);
  await viewer.loadFile(bytes.slice(0));
  const openMs = performance.now() - t0;

  const count = viewer.getSlideCount() || 1;
  const index = Math.min(Math.max(0, slide), Math.max(0, count - 1));
  const t1 = performance.now();
  viewer.goToSlide(index);
  flushLayout(mount);
  const renderMs = performance.now() - t1;

  return { fetchMs: 0, openMs, renderMs, totalMs: openMs + renderMs, slideCount: count };
}

const ENGINES: Record<EngineId, { label: string; run: typeof renderOurs }> = {
  ours: { label: 'pptx-wasm (this project)', run: renderOurs },
  'pptx-preview': { label: 'pptx-preview 1.0.7', run: renderPptxPreview },
  pptxviewjs: { label: 'pptxviewjs 1.1.9', run: renderPptxViewJs },
  aiden0z: { label: '@aiden0z/pptx-renderer 1.2.4', run: renderAiden },
  jvmr: { label: '@jvmr/pptx-to-html 1.1.1', run: renderJvmr },
  glimpse: { label: 'pptx-glimpse 5.0.0', run: renderGlimpse },
  vanilla: { label: 'pptx-vanilla-viewer 1.6.2', run: renderVanilla },
};

/** Every engine, in display order. Kept next to ENGINES so the two cannot drift. */
const ENGINE_IDS = Object.keys(ENGINES) as EngineId[];

// --------------------------------------------------------------------- interactive

function Pane({
  engine,
  bytes,
  slide,
  onResult,
}: {
  engine: EngineId;
  bytes: ArrayBuffer | null;
  slide: number;
  onResult?: (engine: EngineId, timing: Timing | null, error: string | null) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLDivElement>(null);

  // The render stays at W x H; only its presentation shrinks to whatever column width the
  // grid gives this pane, so all four engines fit on one screen without any of them being
  // measured at a different size from the others.
  useEffect(() => {
    const el = frame.current;
    if (!el) return;
    const fit = () => {
      const scale = el.clientWidth / W;
      el.style.setProperty('--pane-scale', String(scale > 0 ? scale : 1));
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const [timing, setTiming] = useState<Timing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const el = host.current;
    if (!el || !bytes) return;

    setBusy(true);
    setError(null);

    ENGINES[engine]
      .run(el, bytes, slide)
      .then((t) => {
        if (cancelled) return;
        setTiming(t);
        setBusy(false);
        onResult?.(engine, t, null);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        // One engine failing must not take the comparison down — that failure is itself
        // a result, and the most interesting one when testing a real deck.
        const message = e instanceof Error ? e.message : String(e);
        el.replaceChildren();
        setError(message);
        setTiming(null);
        setBusy(false);
        onResult?.(engine, null, message);
      });

    return () => {
      cancelled = true;
    };
  }, [engine, bytes, slide, onResult]);

  return (
    <section style={{ minWidth: 0 }}>
      <header style={{ marginBottom: 6 }}>
        <strong>{ENGINES[engine].label}</strong>
        <div style={{ color: error ? '#b00020' : '#666', fontSize: 13, minHeight: 34 }}>
          {error ? (
            <>could not render: {error}</>
          ) : busy ? (
            'rendering…'
          ) : timing ? (
            <>
              open {timing.openMs.toFixed(1)}ms · render {timing.renderMs.toFixed(1)}ms ·{' '}
              <strong>total {timing.totalMs.toFixed(1)}ms</strong>
              <br />
              {timing.slideCount} slide{timing.slideCount === 1 ? '' : 's'}
            </>
          ) : (
            'waiting for a file'
          )}
        </div>
      </header>
      {/*
        Every engine still renders at the full W x H — shrinking the render would change
        what is being compared, and text layout is the first thing that would move. Only
        the *display* is scaled, so four engines fit side by side on one screen.
      */}
      <div
        ref={frame}
        style={{
          width: '100%',
          aspectRatio: `${W} / ${H}`,
          overflow: 'hidden',
          border: '1px solid #ddd',
          background: '#fff',
        }}
      >
        <div
          ref={host}
          data-engine={engine}
          style={{
            width: W,
            height: H,
            transform: `scale(var(--pane-scale, 1))`,
            transformOrigin: 'top left',
            background: '#fff',
            overflow: 'hidden',
            position: 'relative',
            display: 'grid',
            placeItems: 'center',
            color: '#bbb',
          }}
        >
          {!bytes && 'no file'}
        </div>
      </div>
    </section>
  );
}

interface Source {
  bytes: ArrayBuffer;
  label: string;
  /** True when the user supplied it, so we can say so in the UI. */
  custom: boolean;
}

function App() {
  const [source, setSource] = useState<Source | null>(null);
  const [slide, setSlide] = useState(0);
  const [fixture, setFixture] = useState(fixtureParam);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [results, setResults] = useState<
    Partial<Record<EngineId, { timing: Timing | null; error: string | null }>>
  >({});

  const onResult = useCallback((engine: EngineId, timing: Timing | null, error: string | null) => {
    setResults((prev) => ({ ...prev, [engine]: { timing, error } }));
  }, []);

  const loadFixture = useCallback(async (name: string) => {
    setLoadError(null);
    try {
      const { bytes } = await fetchFixture(name);
      setSource({ bytes, label: name, custom: false });
      setSlide(0);
      setResults({});
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const loadFile = useCallback(async (file: File) => {
    setLoadError(null);
    if (!file.name.toLowerCase().endsWith('.pptx')) {
      setLoadError(`${file.name} is not a .pptx file`);
      return;
    }
    try {
      const bytes = await file.arrayBuffer();
      setSource({ bytes, label: file.name, custom: true });
      setSlide(0);
      setResults({});
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  // Start on a fixture so the page is not empty.
  useEffect(() => {
    void loadFixture(fixtureParam);
  }, [loadFixture]);

  // Slide count can differ between engines — that disagreement is itself a finding, so
  // navigation is bounded by the larger of the two and each pane clamps its own.
  const counts = ENGINE_IDS.map((e) => results[e]?.timing?.slideCount).filter(
    (n): n is number => typeof n === 'number',
  );
  const maxSlides = counts.length > 0 ? Math.max(...counts) : 1;
  const disagree = counts.length > 1 && new Set(counts).size > 1;

  const ours = results.ours?.timing;
  // Compared against the *fastest* competitor, not a chosen one — beating the weakest
  // engine in the field is not a claim worth putting on screen.
  const rivals = ENGINE_IDS.filter((e) => e !== 'ours')
    .map((e) => results[e]?.timing?.totalMs)
    .filter((n): n is number => typeof n === 'number');
  const best = rivals.length > 0 ? Math.min(...rivals) : null;
  const speedup = ours && best != null ? best / Math.max(ours.totalMs, 0.001) : null;

  return (
    <main
      style={{ font: '14px system-ui, sans-serif', padding: 20 }}
      onDragOver={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragging(false);
        const file = e.dataTransfer.files?.[0];
        if (file) void loadFile(file);
      }}
    >
      <h1 style={{ fontSize: 19, marginTop: 0 }}>pptx renderer comparison</h1>
      <p style={{ color: '#555', maxWidth: 780, marginTop: 0 }}>
        Same bytes, same slide, same pixel size, rendered by every engine. Timings here are
        indicative only — the engines share a page and each other's warm caches, so they
        flatter whichever loads last. For measured numbers run{' '}
        <code>npm run compare</code>, which loads each engine in its own page and scores
        accuracy against LibreOffice.
      </p>

      <div
        style={{
          display: 'flex',
          gap: 14,
          alignItems: 'center',
          flexWrap: 'wrap',
          marginBottom: 14,
          padding: 12,
          border: dragging ? '2px dashed #4472C4' : '1px solid #e3e3e3',
          borderRadius: 6,
          background: dragging ? '#f2f6fd' : '#fafafa',
        }}
      >
        <label>
          <strong>Your deck:</strong>{' '}
          <input
            type="file"
            accept=".pptx,application/vnd.openxmlformats-officedocument.presentationml.presentation"
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) void loadFile(file);
            }}
          />
        </label>
        <span style={{ color: '#888' }}>or drop one anywhere on this page</span>

        <span style={{ borderLeft: '1px solid #ddd', height: 20 }} />

        <label>
          Fixture{' '}
          <select
            value={fixture}
            onChange={(e) => {
              setFixture(e.target.value);
              void loadFixture(e.target.value);
            }}
          >
            {FIXTURES.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
          </select>
        </label>
      </div>

      {loadError && (
        <p role="alert" style={{ color: '#b00020' }}>
          {loadError}
        </p>
      )}

      <div style={{ display: 'flex', gap: 12, alignItems: 'center', marginBottom: 16 }}>
        <button onClick={() => setSlide((s) => Math.max(0, s - 1))} disabled={slide === 0}>
          ‹ Previous
        </button>
        <span style={{ minWidth: 110, textAlign: 'center' }}>
          Slide {slide + 1} of {maxSlides}
        </span>
        <button
          onClick={() => setSlide((s) => Math.min(maxSlides - 1, s + 1))}
          disabled={slide >= maxSlides - 1}
        >
          Next ›
        </button>

        <span style={{ color: '#666' }}>
          {source ? (
            <>
              {source.custom ? '📄 ' : ''}
              <code>{source.label}</code>
            </>
          ) : (
            'loading…'
          )}
        </span>

        {speedup && (
          <span
            style={{
              marginLeft: 'auto',
              padding: '4px 10px',
              borderRadius: 4,
              background: speedup >= 1 ? '#e8f5e9' : '#fdecea',
              color: speedup >= 1 ? '#1b5e20' : '#b00020',
            }}
          >
            {speedup >= 1
              ? `pptx-wasm is ${speedup.toFixed(1)}× faster than the next best on this slide`
              : `the fastest other engine is ${(1 / speedup).toFixed(1)}× faster on this slide`}
          </span>
        )}
      </div>

      {disagree && (
        <p style={{ color: '#8a6d00', background: '#fff8e1', padding: '8px 12px', borderRadius: 4 }}>
          The engines disagree about how many slides this deck has ({counts.join(' vs ')}).
          That is worth investigating — one of them is dropping or inventing a slide.
        </p>
      )}

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(480px, 1fr))', gap: 20 }}>
        {ENGINE_IDS.map((id) => (
          <Pane
            key={id}
            engine={id}
            bytes={source?.bytes ?? null}
            slide={slide}
            onResult={onResult}
          />
        ))}
      </div>

      <p style={{ color: '#777', marginTop: 20, maxWidth: 780 }}>
        Nothing is uploaded. Every engine runs entirely in this page, so you can compare a
        confidential deck without it leaving your machine.
      </p>
    </main>
  );
}

// --------------------------------------------------------------------- headless

function Headless() {
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = host.current;
    if (!el) return;

    const runOnce = async (slide = slideParam): Promise<Timing> => {
      const { bytes, fetchMs } = await fetchFixture(fixtureParam);
      const result = await ENGINES[engineParam].run(el, bytes, slide);
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

const root = document.getElementById('root');
if (root) createRoot(root).render(headless ? <Headless /> : <App />);
