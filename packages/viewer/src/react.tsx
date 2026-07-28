/**
 * The React wrapper.
 *
 * ```tsx
 * <PresentationViewer src="/file.pptx" width="100%" height="100vh" />
 * ```
 *
 * The component owns three things the API layer deliberately does not: when to re-render
 * (a resize, a slide change, a prop change), how to keep the canvas backing store in step
 * with its CSS size, and keyboard navigation.
 */

import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react';

import { Presentation } from './presentation.js';
import {
  PptxError,
  type PresentationInfo,
  type PresentationViewerProps,
  type TextLayerRun,
} from './types.js';

/** Imperative handle, for callers that need to drive the viewer. */
export interface PresentationViewerHandle {
  next(): void;
  previous(): void;
  goTo(index: number): void;
  readonly slide: number;
  readonly slideCount: number;
  /** The open presentation, once loaded. */
  readonly presentation: Presentation | null;
  /** Redraw the current slide. */
  redraw(): void;
}

export const PresentationViewer = forwardRef<PresentationViewerHandle, PresentationViewerProps>(
  function PresentationViewer(props, ref) {
    const {
      src,
      width = '100%',
      height = '100%',
      initialSlide = 0,
      slide: controlledSlide,
      fit = 'contain',
      zoom = 1,
      keyboard = true,
      selectableText = false,
      wasm,
      loading,
      renderError,
      className,
      style,
      onLoad,
      onError,
      onSlideChange,
    } = props;

    const canvasRef = useRef<HTMLCanvasElement | null>(null);
    const [textRuns, setTextRuns] = useState<TextLayerRun[]>([]);
    const containerRef = useRef<HTMLDivElement | null>(null);
    const presentationRef = useRef<Presentation | null>(null);

    const [info, setInfo] = useState<PresentationInfo | null>(null);
    const [error, setError] = useState<PptxError | null>(null);
    const [uncontrolledSlide, setUncontrolledSlide] = useState(initialSlide);

    const isControlled = controlledSlide !== undefined;
    const currentSlide = isControlled ? controlledSlide : uncontrolledSlide;

    // Callbacks are held in a ref so that a caller passing an inline arrow function does
    // not re-open the deck on every render.
    const handlers = useRef({ onLoad, onError, onSlideChange });
    handlers.current = { onLoad, onError, onSlideChange };

    // --- open the deck -----------------------------------------------------------

    useEffect(() => {
      let cancelled = false;
      const controller = new AbortController();
      setError(null);
      setInfo(null);

      Presentation.open(src, { wasm, signal: controller.signal })
        .then((deck) => {
          if (cancelled) {
            deck.destroy();
            return;
          }
          presentationRef.current = deck;
          setInfo(deck.info);
          handlers.current.onLoad?.(deck.info);
        })
        .catch((e: unknown) => {
          if (cancelled) return;
          const err = e instanceof PptxError ? e : new PptxError(String(e), e);
          setError(err);
          handlers.current.onError?.(err);
        });

      return () => {
        cancelled = true;
        controller.abort();
        presentationRef.current?.destroy();
        presentationRef.current = null;
      };
    }, [src, wasm]);

    // Clamp the slide index when a shorter deck loads under a stale index.
    useEffect(() => {
      if (!info || isControlled) return;
      if (uncontrolledSlide >= info.slideCount) {
        setUncontrolledSlide(Math.max(0, info.slideCount - 1));
      }
    }, [info, isControlled, uncontrolledSlide]);

    // --- drawing -----------------------------------------------------------------

    const draw = useCallback(() => {
      const deck = presentationRef.current;
      const canvas = canvasRef.current;
      if (!deck || !canvas || deck.slideCount === 0) return;
      const index = Math.min(Math.max(0, currentSlide), deck.slideCount - 1);

      // A frame that was missing images is redrawn from the `onAssetsReady` subscription
      // below, not from a guessed delay here.
      deck.render(index, canvas, { fit, zoom }).catch((e: unknown) => {
        const err = e instanceof PptxError ? e : new PptxError(String(e), e);
        setError(err);
        handlers.current.onError?.(err);
      });

      // The overlay is positioned in CSS pixels, so it asks for the layout at the
      // canvas's CSS size with dpr 1 — the same view, minus the device-pixel scaling.
      if (selectableText) {
        setTextRuns(
          deck.textLayer(index, {
            width: canvas.clientWidth || undefined,
            height: canvas.clientHeight || undefined,
            dpr: 1,
            fit,
            zoom,
          }),
        );
      }

      // Warm the neighbours so navigation does not pay for layout.
      deck.prepare(index + 1);
      deck.prepare(index - 1);
    }, [currentSlide, fit, zoom, selectableText]);

    useEffect(() => {
      draw();
    }, [draw, info]);

    // Images decode after the first frame. Redraw when they land, rather than polling or
    // guessing how long decoding takes.
    useEffect(() => {
      const deck = presentationRef.current;
      if (!deck) return;
      return deck.onAssetsReady(draw);
    }, [draw, info]);

    // Redraw on resize. `ResizeObserver` rather than a window listener, so a viewer in a
    // resizable panel stays correct without the page resizing at all.
    useEffect(() => {
      const container = containerRef.current;
      if (!container || typeof ResizeObserver === 'undefined') return;
      let frame = 0;
      const observer = new ResizeObserver(() => {
        // Coalesce: a drag-resize fires this continuously and each draw is a full render.
        cancelAnimationFrame(frame);
        frame = requestAnimationFrame(draw);
      });
      observer.observe(container);
      return () => {
        cancelAnimationFrame(frame);
        observer.disconnect();
      };
    }, [draw]);

    // Redraw when the device pixel ratio changes — dragging a window between a Retina
    // and a non-Retina display does not fire a resize.
    useEffect(() => {
      if (typeof window === 'undefined' || !window.matchMedia) return;
      const query = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      const onChange = () => draw();
      query.addEventListener?.('change', onChange);
      return () => query.removeEventListener?.('change', onChange);
    }, [draw]);

    // --- navigation --------------------------------------------------------------

    const goTo = useCallback(
      (index: number) => {
        const count = presentationRef.current?.slideCount ?? 0;
        if (count === 0) return;
        const clamped = Math.min(Math.max(0, index), count - 1);
        if (clamped === currentSlide) return;
        if (!isControlled) setUncontrolledSlide(clamped);
        handlers.current.onSlideChange?.(clamped);
      },
      [currentSlide, isControlled],
    );

    const next = useCallback(() => goTo(currentSlide + 1), [goTo, currentSlide]);
    const previous = useCallback(() => goTo(currentSlide - 1), [goTo, currentSlide]);

    useEffect(() => {
      if (!keyboard) return;
      const container = containerRef.current;
      if (!container) return;
      const onKeyDown = (e: KeyboardEvent) => {
        switch (e.key) {
          case 'ArrowRight':
          case 'ArrowDown':
          case 'PageDown':
          case ' ':
            next();
            break;
          case 'ArrowLeft':
          case 'ArrowUp':
          case 'PageUp':
            previous();
            break;
          case 'Home':
            goTo(0);
            break;
          case 'End':
            goTo((presentationRef.current?.slideCount ?? 1) - 1);
            break;
          default:
            return;
        }
        // Only prevent default for keys actually handled, so Tab and shortcuts still work.
        e.preventDefault();
      };
      container.addEventListener('keydown', onKeyDown);
      return () => container.removeEventListener('keydown', onKeyDown);
    }, [keyboard, next, previous, goTo]);

    useImperativeHandle(
      ref,
      () => ({
        next,
        previous,
        goTo,
        get slide() {
          return currentSlide;
        },
        get slideCount() {
          return presentationRef.current?.slideCount ?? 0;
        },
        get presentation() {
          return presentationRef.current;
        },
        redraw: draw,
      }),
      [next, previous, goTo, currentSlide, draw],
    );

    // --- render ------------------------------------------------------------------

    const containerStyle = useMemo<React.CSSProperties>(
      () => ({
        width: typeof width === 'number' ? `${width}px` : width,
        height: typeof height === 'number' ? `${height}px` : height,
        position: 'relative',
        overflow: 'hidden',
        outline: 'none',
        ...style,
      }),
      [width, height, style],
    );

    const slideText = info ? (presentationRef.current?.text(currentSlide) ?? '') : '';

    return (
      <div
        ref={containerRef}
        className={className}
        style={containerStyle}
        tabIndex={keyboard ? 0 : undefined}
        role="region"
        aria-roledescription="presentation viewer"
        aria-label={
          info ? `Slide ${currentSlide + 1} of ${info.slideCount}` : 'Loading presentation'
        }
      >
        {error ? (
          (renderError?.(error) ?? <DefaultError error={error} />)
        ) : (
          <>
            <canvas
              ref={canvasRef}
              style={{ display: 'block', width: '100%', height: '100%' }}
              // The canvas is decorative; the text below carries the content for
              // assistive technology and for find-in-page.
              aria-hidden="true"
            />
            {selectableText && (
              // Transparent but selectable. `left`/`top` are the run's baseline, so the
              // span is shifted up by its own size to sit on it, and scaleX corrects the
              // browser's advance width to the one layout actually measured — without
              // that the selection drifts from the glyphs across a long line.
              <div
                aria-hidden="true"
                style={{
                  position: 'absolute',
                  inset: 0,
                  overflow: 'hidden',
                  pointerEvents: 'none',
                  userSelect: 'text',
                }}
              >
                {textRuns.map((r, i) => (
                  <span
                    key={i}
                    style={{
                      position: 'absolute',
                      left: r.x,
                      top: r.y - r.size,
                      height: r.size,
                      fontFamily: r.family,
                      fontSize: r.size,
                      fontWeight: r.weight,
                      fontStyle: r.italic ? 'italic' : 'normal',
                      lineHeight: 1,
                      whiteSpace: 'pre',
                      color: 'transparent',
                      transformOrigin: '0% 0%',
                      pointerEvents: 'auto',
                      cursor: 'text',
                    }}
                    ref={(el) => fitSpan(el, r.width, r.rotation)}
                  >
                    {r.text}
                  </span>
                ))}
              </div>
            )}
            {!info && (loading ?? <DefaultLoading />)}
            {/* Visually hidden, but present in the accessibility tree and searchable. */}
            <div
              style={{
                position: 'absolute',
                width: 1,
                height: 1,
                overflow: 'hidden',
                clip: 'rect(0 0 0 0)',
                clipPath: 'inset(50%)',
                whiteSpace: 'nowrap',
              }}
            >
              {slideText}
            </div>
          </>
        )}
      </div>
    );
  },
);

/**
 * Squeezes a span to the width layout measured for the run.
 *
 * The browser re-measures the string itself, and its advance will not match ours exactly —
 * a different fallback face, different kerning. Left alone the two drift apart across a
 * long line and the selection highlight stops matching the glyphs under it. Scaling the
 * span to the measured width pins them together.
 */
function fitSpan(el: HTMLSpanElement | null, width: number, rotation: number): void {
  if (!el || width <= 0) return;
  // Rebuilt from the run every time rather than edited in place: React re-runs this ref
  // on each render, and appending to whatever is already there compounds the correction.
  const base = rotation ? `rotate(${rotation}rad)` : '';
  el.style.transform = base;
  const actual = el.getBoundingClientRect().width;
  if (actual > 0.5) {
    el.style.transform = `${base} scaleX(${width / actual})`.trim();
  }
}

function DefaultLoading() {
  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        display: 'grid',
        placeItems: 'center',
        font: '14px system-ui, sans-serif',
        color: '#666',
      }}
    >
      Loading…
    </div>
  );
}

function DefaultError({ error }: { error: PptxError }) {
  return (
    <div
      role="alert"
      style={{
        position: 'absolute',
        inset: 0,
        display: 'grid',
        placeItems: 'center',
        padding: 16,
        font: '14px system-ui, sans-serif',
        color: '#b00020',
        textAlign: 'center',
      }}
    >
      {error.message}
    </div>
  );
}

export type { PresentationViewerProps } from './types.js';
