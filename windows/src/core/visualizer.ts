// Real-time music audio visualizer state and band smoothing.
// Receives 16 FFT frequency bands from Rust (WASAPI loopback capture).

import { Bridge, onEvent } from "./bridge";

export const VISUALIZER_BANDS = 16;

class MusicVisualizerManager {
  private active = false;
  private rawLevels = new Array<number>(VISUALIZER_BANDS).fill(0);
  private easedLevels = new Array<number>(VISUALIZER_BANDS).fill(0);
  private listeners = new Set<(bands: number[]) => void>();
  private initialized = false;
  private lastTick = performance.now();
  private rafId: number | null = null;

  init() {
    if (this.initialized) return;
    this.initialized = true;

    void onEvent<number[]>("music-bands", (bands) => {
      if (!Array.isArray(bands) || bands.length === 0) return;
      for (let i = 0; i < VISUALIZER_BANDS; i++) {
        this.rawLevels[i] = bands[i] ?? 0;
      }
      this.ensureLoop();
    });
  }

  /** Starts or stops the background audio capture based on playback & visibility. */
  setActive(active: boolean) {
    if (this.active === active) return;
    this.active = active;
    void Bridge.musicVisualizerSetActive(active);

    if (active) {
      this.ensureLoop();
    } else {
      for (let i = 0; i < VISUALIZER_BANDS; i++) {
        this.rawLevels[i] = 0;
      }
    }
  }

  get isActive(): boolean {
    return this.active;
  }

  /** 16 smoothed frequency bands (0.0 .. 1.0). */
  get bands(): readonly number[] {
    return this.easedLevels;
  }

  /** 4 frequency bands for the closed island (Bass, Low-Mid, High-Mid, Treble). */
  get closedBands(): [number, number, number, number] {
    const avg = (start: number, end: number) => {
      let sum = 0;
      for (let i = start; i < end; i++) sum += this.easedLevels[i] ?? 0;
      return sum / (end - start);
    };
    return [
      avg(0, 4),
      avg(4, 8),
      avg(8, 12),
      avg(12, 16),
    ];
  }

  subscribe(listener: (bands: number[]) => void): () => void {
    this.listeners.add(listener);
    this.ensureLoop();
    return () => {
      this.listeners.delete(listener);
    };
  }

  private ensureLoop() {
    if (this.rafId != null) return;
    this.lastTick = performance.now();
    this.tick = this.tick.bind(this);
    this.rafId = requestAnimationFrame(this.tick);
  }

  private tick(now: number) {
    this.rafId = null;
    const dt = Math.min(0.05, (now - this.lastTick) / 1000);
    this.lastTick = now;

    let hasEnergy = false;
    for (let i = 0; i < VISUALIZER_BANDS; i++) {
      const target = this.rawLevels[i] ?? 0;
      const speed = target > this.easedLevels[i] ? 20 : 12;
      this.easedLevels[i] += (target - this.easedLevels[i]) * Math.min(1, dt * speed);
      if (this.easedLevels[i] > 0.005) {
        hasEnergy = true;
      }
    }

    for (const listener of this.listeners) {
      listener(this.easedLevels);
    }

    if (this.active || hasEnergy) {
      this.rafId = requestAnimationFrame(this.tick);
    }
  }
}

export const MusicVisualizer = new MusicVisualizerManager();
