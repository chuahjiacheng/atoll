// Island metrics: see docs/contracts/metrics.md.
// What the island is showing, how its shape is moving, and its frame times,
// for the dev overlay and the test harness. Nothing here runs on its own:
// numbers are only updated by frames the island was drawing anyway, so at
// rest this costs nothing.
import { clock, type SpringFrame } from "./spring";

export interface FrameStats {
  budgetMs: number;
  count: number;
  dropped: number;
  p50: number;
  p95: number;
  max: number;
  longestJumpPx: number;
  durationMs: number;
}

export interface StateInfo {
  id: string;
  previous: string | null;
  since: number;
  reason: string;
  queue: { id: string; source: string; at: number }[];
}

export interface IslandMetrics {
  version: 1;
  at: number;
  state: StateInfo;
  width: { current: number; target: number; velocity: number; min: number; max: number; settled: boolean; retargets: number };
  height: { current: number; target: number; settled: boolean };
  radius: number;
  loopRunning: boolean;
  frames: FrameStats;
  lyrics: null | { mode: "follow" | "smooth" | "fixed"; line: number; nextChangeIn: number; lookaheadTarget: number };
}

export interface IslandMetricsApi {
  snapshot(): IslandMetrics;
  subscribe(cb: (m: IslandMetrics) => void): () => void;
  begin(label: string): void;
  end(label: string): FrameStats;
  reset(): void;
}

/** Common display refresh rates; the frame budget snaps to the nearest. */
const RATES = [60, 72, 75, 90, 100, 120, 144, 165, 240];
const SAMPLE_LIMIT = 20_000;

class Recorder {
  deltas: number[] = [];
  dropped = 0;
  longestJump = 0;
  duration = 0;

  add(delta: number, jump: number, budget: number) {
    if (this.deltas.length < SAMPLE_LIMIT) this.deltas.push(delta);
    if (delta > budget * 1.5) this.dropped++;
    if (jump > this.longestJump) this.longestJump = jump;
    this.duration += delta;
  }

  stats(budget: number): FrameStats {
    const sorted = [...this.deltas].sort((a, b) => a - b);
    const at = (q: number) => (sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] : 0);
    const round = (n: number) => Math.round(n * 100) / 100;
    return {
      budgetMs: round(budget),
      count: sorted.length,
      dropped: this.dropped,
      p50: round(at(0.5)),
      p95: round(at(0.95)),
      max: round(sorted[sorted.length - 1] ?? 0),
      longestJumpPx: round(this.longestJump),
      durationMs: round(this.duration),
    };
  }
}

class Metrics implements IslandMetricsApi {
  private state: StateInfo = { id: "idle", previous: null, since: clock(), reason: "starting", queue: [] };
  private limits = { min: 0, max: 0 };
  private frame: SpringFrame | null = null;
  private retargets = 0;
  private loopRunning = false;
  private lastWidth: number | null = null;
  private budget = 1000 / 60;
  /** One frame at the display's refresh rate, as learned so far (ms). */
  get budgetMs() {
    return this.budget;
  }
  private recent: number[] = [];
  private total = new Recorder();
  private recordings = new Map<string, Recorder>();
  private subscribers = new Set<(m: IslandMetrics) => void>();
  private frameListeners = new Set<() => void>();
  lyrics: IslandMetrics["lyrics"] = null;

  // ---------- written by the island ----------

  setState(id: string, reason: string, queue: StateInfo["queue"] = []) {
    if (id === this.state.id && reason === this.state.reason) return;
    this.state = { id, previous: id === this.state.id ? this.state.previous : this.state.id, since: clock(), reason, queue };
    this.emit();
  }

  setLimits(min: number, max: number) {
    this.limits = { min, max };
  }

  /** One frame of the shape spring. */
  onFrame(f: SpringFrame, retargets: number) {
    this.retargets = retargets;
    this.loopRunning = !f.rest;
    if (f.delta > 0) {
      this.learnBudget(f.delta);
      const jump = this.lastWidth === null ? 0 : Math.abs(f.pos.w - this.lastWidth);
      this.total.add(f.delta, jump, this.budget);
      this.recordings.forEach((r) => r.add(f.delta, jump, this.budget));
    }
    this.lastWidth = f.pos.w;
    this.frame = f;
    this.frameListeners.forEach((cb) => cb());
    if (f.rest) this.emit();
  }

  /** Dev overlay: called on every frame the island draws (never on its own timer). */
  onEveryFrame(cb: () => void): () => void {
    this.frameListeners.add(cb);
    return () => this.frameListeners.delete(cb);
  }

  // ---------- IslandMetricsApi ----------

  snapshot(): IslandMetrics {
    const f = this.frame;
    return {
      version: 1,
      at: clock(),
      state: this.state,
      width: {
        current: f?.pos.w ?? 0,
        target: f?.target.w ?? 0,
        velocity: (f?.vel.w ?? 0) * 1000,
        min: this.limits.min,
        max: this.limits.max,
        settled: !this.loopRunning,
        retargets: this.retargets,
      },
      height: { current: f?.pos.h ?? 0, target: f?.target.h ?? 0, settled: !this.loopRunning },
      radius: f?.pos.r ?? 0,
      loopRunning: this.loopRunning,
      frames: this.total.stats(this.budget),
      lyrics: this.lyrics,
    };
  }

  subscribe(cb: (m: IslandMetrics) => void): () => void {
    this.subscribers.add(cb);
    return () => this.subscribers.delete(cb);
  }

  begin(label: string) {
    this.recordings.set(label, new Recorder());
  }

  end(label: string): FrameStats {
    const r = this.recordings.get(label) ?? new Recorder();
    this.recordings.delete(label);
    return r.stats(this.budget);
  }

  reset() {
    this.total = new Recorder();
  }

  // The display's refresh rate is not exposed to pages, so it is learned from
  // the frames themselves: the median of recent frame times, snapped to a real rate.
  private learnBudget(delta: number) {
    this.recent.push(delta);
    if (this.recent.length > 90) this.recent.shift();
    if (this.recent.length < 12) return;
    const mid = [...this.recent].sort((a, b) => a - b)[Math.floor(this.recent.length / 2)];
    const rate = RATES.reduce((best, r) => (Math.abs(1000 / r - mid) < Math.abs(1000 / best - mid) ? r : best), 60);
    this.budget = 1000 / rate;
  }

  private emit() {
    if (!this.subscribers.size) return;
    const m = this.snapshot();
    this.subscribers.forEach((cb) => cb(m));
  }
}

export const metrics = new Metrics();

declare global {
  interface Window {
    __atoll?: { metrics: IslandMetricsApi };
  }
}

/** Expose the read-only API to the test harness (debug builds only). */
export function exposeMetrics() {
  if (!import.meta.env.DEV || typeof window === "undefined") return;
  window.__atoll = {
    metrics: {
      snapshot: () => metrics.snapshot(),
      subscribe: (cb) => metrics.subscribe(cb),
      begin: (label) => metrics.begin(label),
      end: (label) => metrics.end(label),
      reset: () => metrics.reset(),
    },
  };
}
