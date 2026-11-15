// The island's shape spring: width, height and corner radius move together on
// one critically damped spring (no overshoot from rest).
//
// It is solved in closed form, not stepped: the shape at any time is a pure
// function of a small "segment" (where it started, how fast it was moving,
// where it is going, how stiff). That gives three things a tween cannot:
//
// - Retargeting mid-flight keeps the current position AND velocity, so there
//   is no jump or kink when the target changes during a move.
// - Frame rate does not matter: a late frame lands exactly where it should.
// - The same segment can be evaluated anywhere else (the native glass and
//   hit region, step 2) and gives the same rectangle for the same time.
//
// No DOM in the maths, so it can be unit tested (spring.test.ts).

export interface Shape {
  w: number;
  h: number;
  r: number;
}

/** Matches ShapeSegment in docs/contracts/width-contract.md. Times in ms, omega in 1/ms. */
export interface ShapeSegment {
  t0: number;
  from: Shape;
  /** px per ms at t0 */
  velocity: Shape;
  to: Shape;
  omega: number;
}

const KEYS = ["w", "h", "r"] as const;
/** At rest once every channel is this close (px) and this slow (px per ms). */
export const REST_DISTANCE = 0.25;
export const REST_SPEED = 0.005;

/** The shared clock, ms: the same time base the frame callbacks use. */
export function clock(): number {
  return performance.timeOrigin + performance.now();
}

export function still(at: Shape, t = 0): ShapeSegment {
  return { t0: t, from: { ...at }, velocity: { w: 0, h: 0, r: 0 }, to: { ...at }, omega: 0.02 };
}

/** Position and velocity of the shape at time `t`. */
export function evaluate(seg: ShapeSegment, t: number): { pos: Shape; vel: Shape } {
  const dt = Math.max(0, t - seg.t0);
  const decay = Math.exp(-seg.omega * dt);
  const pos = { w: 0, h: 0, r: 0 };
  const vel = { w: 0, h: 0, r: 0 };
  for (const k of KEYS) {
    const c1 = seg.from[k] - seg.to[k];
    const c2 = seg.velocity[k] + seg.omega * c1;
    const body = c1 + c2 * dt;
    pos[k] = seg.to[k] + body * decay;
    vel[k] = (c2 - seg.omega * body) * decay;
  }
  return { pos, vel };
}

export function atRest(seg: ShapeSegment, t: number): boolean {
  const { pos, vel } = evaluate(seg, t);
  return KEYS.every((k) => Math.abs(pos[k] - seg.to[k]) < REST_DISTANCE && Math.abs(vel[k]) < REST_SPEED);
}

/**
 * A new segment towards `to`, starting from wherever `seg` is at `t`.
 * Position always carries over exactly (no jump). Velocity carries over too,
 * with one limit: speed TOWARDS the new target is capped at omega × distance,
 * the most a critically damped spring can absorb without passing its target.
 * So a fast move that is retargeted to somewhere close slows down harder
 * instead of sailing past and coming back (DESIGN.md: motion settles, it
 * does not overshoot). Speed away from the new target is kept in full and
 * turns around smoothly.
 */
export function retarget(seg: ShapeSegment, to: Shape, omega: number, t: number): ShapeSegment {
  const { pos, vel } = evaluate(seg, t);
  for (const k of KEYS) {
    const gap = to[k] - pos[k];
    const limit = omega * Math.abs(gap);
    if (gap >= 0 && vel[k] > limit) vel[k] = limit;
    else if (gap < 0 && vel[k] < -limit) vel[k] = -limit;
  }
  return { t0: t, from: pos, velocity: vel, to: { ...to }, omega };
}

export interface SpringFrame {
  pos: Shape;
  vel: Shape;
  target: Shape;
  /** ms since the previous frame of this run; 0 on the first. */
  delta: number;
  /** True on the last frame of a run (the shape is exactly on target). */
  rest: boolean;
}

/**
 * Runs a segment on requestAnimationFrame. The loop exists only while the
 * shape is moving: at rest nothing is scheduled at all.
 */
export class ShapeSpring {
  segment: ShapeSegment;
  running = false;
  /** Retargets since the shape was last at rest. */
  retargets = 0;
  private raf = 0;
  private last = 0;
  private onFrame: (f: SpringFrame) => void;

  constructor(at: Shape, onFrame: (f: SpringFrame) => void) {
    this.segment = still(at, clock());
    this.onFrame = onFrame;
  }

  get target(): Shape {
    return this.segment.to;
  }

  /** Where the shape is right now. */
  now(): { pos: Shape; vel: Shape } {
    return evaluate(this.segment, clock());
  }

  /** Move to `to`. `omega` in 1/s (settles to 1 % in about 6.6 / omega seconds). */
  to(to: Shape, omegaPerSecond: number) {
    const t = clock();
    if (this.running) this.retargets++;
    this.segment = retarget(this.segment, to, omegaPerSecond / 1000, t);
    if (atRest(this.segment, t)) {
      this.jump(to);
      return;
    }
    if (!this.running) {
      this.running = true;
      this.last = 0;
      this.raf = requestAnimationFrame(this.step);
    }
  }

  /** Be at `to` now, with no animation (first paint, reduced motion). */
  jump(to: Shape) {
    cancelAnimationFrame(this.raf);
    this.running = false;
    this.retargets = 0;
    this.segment = still(to, clock());
    this.onFrame({ pos: { ...to }, vel: { w: 0, h: 0, r: 0 }, target: { ...to }, delta: 0, rest: true });
  }

  stop() {
    cancelAnimationFrame(this.raf);
    this.running = false;
  }

  private step = (stamp: number) => {
    // rAF's timestamp is the frame's start on the performance.now() clock.
    const t = performance.timeOrigin + stamp;
    const delta = this.last ? t - this.last : 0;
    this.last = t;
    if (atRest(this.segment, t)) {
      const to = this.segment.to;
      this.running = false;
      this.retargets = 0;
      this.segment = still(to, t);
      this.onFrame({ pos: { ...to }, vel: { w: 0, h: 0, r: 0 }, target: { ...to }, delta, rest: true });
      return;
    }
    const { pos, vel } = evaluate(this.segment, t);
    this.onFrame({ pos, vel, target: this.segment.to, delta, rest: false });
    this.raf = requestAnimationFrame(this.step);
  };
}
