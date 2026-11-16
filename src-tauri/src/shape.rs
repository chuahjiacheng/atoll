//! The island's shape, as a function of time.
//!
//! The island page moves its outline on a spring (src/lib/island/spring.ts).
//! It used to report its rectangle after every layout, so the glass under it
//! (backdrop.rs, liquid.rs) and the hover test (lib.rs) always trailed the
//! shape they were following. Now the page sends the *description* of each
//! move once, a `Segment`, and both sides work out the same rectangle for any
//! moment from the same formula and the same clock:
//!
//! - the glass draws the rectangle for the frame it is about to show;
//! - the hover test checks the cursor against the rectangle as it is now.
//!
//! No per-frame messages cross between the page and Rust, and nothing here
//! runs on its own: `moving()` is what lets the glass loop exist only while
//! the shape is in motion.
//!
//! Units: CSS px inside the island window, before the island zoom. Times: ms
//! on the native clock (`now_ms`); the page converts its own clock with the
//! offset it measures through `island_clock`.

use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Width, height and corner radius.
#[derive(Clone, Copy, Debug, PartialEq, Default, Deserialize, Serialize)]
pub struct Shape {
    pub w: f64,
    pub h: f64,
    pub r: f64,
}

/// One move of a critically damped spring. Mirrors `ShapeSegment` in spring.ts
/// (docs/contracts/width-contract.md §7): `omega` in 1/ms, `velocity` in px/ms.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Segment {
    pub t0: f64,
    pub from: Shape,
    pub velocity: Shape,
    pub to: Shape,
    pub omega: f64,
}

/// A segment plus where the island sits in its window: it grows and shrinks
/// evenly around `cx`, hanging from `top`.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Motion {
    pub segment: Segment,
    pub cx: f64,
    pub top: f64,
    /// The hover area is never narrower than this (the idle pill is small).
    #[serde(default)]
    pub hover_min: f64,
}

/// A rectangle in the island window, CSS px.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub r: f64,
}

/// At rest once every channel is this close (px) and this slow (px/ms). Same as spring.ts.
const REST_DISTANCE: f64 = 0.25;
const REST_SPEED: f64 = 0.005;

static MOTION: Mutex<Option<Motion>> = Mutex::new(None);
static CLOCK: OnceLock<(Instant, f64)> = OnceLock::new();

/// Milliseconds since the Unix epoch, on a clock that never jumps: the wall
/// clock is read once, and time is counted from there with `Instant`.
pub fn now_ms() -> f64 {
    let (start, base) = CLOCK.get_or_init(|| {
        let epoch = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
        (Instant::now(), epoch)
    });
    base + start.elapsed().as_secs_f64() * 1000.0
}

fn channel(from: f64, velocity: f64, to: f64, omega: f64, dt: f64) -> (f64, f64) {
    let decay = (-omega * dt).exp();
    let c1 = from - to;
    let c2 = velocity + omega * c1;
    let body = c1 + c2 * dt;
    (to + body * decay, (c2 - omega * body) * decay)
}

/// Position and velocity of the shape at time `t`.
pub fn evaluate(seg: &Segment, t: f64) -> (Shape, Shape) {
    let dt = (t - seg.t0).max(0.0);
    let (w, vw) = channel(seg.from.w, seg.velocity.w, seg.to.w, seg.omega, dt);
    let (h, vh) = channel(seg.from.h, seg.velocity.h, seg.to.h, seg.omega, dt);
    let (r, vr) = channel(seg.from.r, seg.velocity.r, seg.to.r, seg.omega, dt);
    (Shape { w, h, r }, Shape { w: vw, h: vh, r: vr })
}

pub fn at_rest(seg: &Segment, t: f64) -> bool {
    let (p, v) = evaluate(seg, t);
    [(p.w - seg.to.w, v.w), (p.h - seg.to.h, v.h), (p.r - seg.to.r, v.r)]
        .iter()
        .all(|(d, s)| d.abs() < REST_DISTANCE && s.abs() < REST_SPEED)
}

fn rect_of(m: &Motion, s: Shape) -> Rect {
    Rect { x: m.cx - s.w / 2.0, y: m.top, w: s.w, h: s.h, r: s.r.min(s.h / 2.0).max(0.0) }
}

/// The exact shape at `t` (the target once the move has settled).
fn shape_at(m: &Motion, t: f64) -> Shape {
    if at_rest(&m.segment, t) { m.segment.to } else { evaluate(&m.segment, t).0 }
}

/// Where the glass should be for a frame shown around `t`.
///
/// The page and the glass are two surfaces that Windows puts on screen
/// separately, so they can land a frame apart. Glass that sticks out past the
/// island's edge is ugly; glass a hair inside it is invisible (the island
/// draws its own hull on top). So the glass takes the smallest size the shape
/// has within one frame either side of `t`: it never bleeds, whichever way
/// the two surfaces are offset.
pub fn glass_rect(t: f64, frame_ms: f64) -> Option<Rect> {
    let m = (*MOTION.lock().unwrap())?;
    let now = shape_at(&m, t);
    let (a, b) = (shape_at(&m, t - frame_ms), shape_at(&m, t + frame_ms));
    Some(rect_of(&m, Shape { w: now.w.min(a.w).min(b.w), h: now.h.min(a.h).min(b.h), r: now.r }))
}

/// The area that counts as "over the island" at `t`: the largest the shape is
/// within one poll either side (so the edge is never missed mid-move), and
/// never narrower than the hover minimum.
pub fn hit_rect(t: f64, poll_ms: f64) -> Option<Rect> {
    let m = (*MOTION.lock().unwrap())?;
    let now = shape_at(&m, t);
    let (a, b) = (shape_at(&m, t - poll_ms), shape_at(&m, t + poll_ms));
    let w = now.w.max(a.w).max(b.w).max(m.hover_min);
    Some(rect_of(&m, Shape { w, h: now.h.max(a.h).max(b.h), r: now.r }))
}

/// Is the shape in motion at `t`? (False when nothing has been sent yet.)
pub fn moving(t: f64) -> bool {
    MOTION.lock().unwrap().as_ref().is_some_and(|m| !at_rest(&m.segment, t))
}

/// The page's clock handshake: it calls this a few times and keeps the offset
/// from the quickest round trip.
#[tauri::command]
pub fn island_clock() -> f64 {
    now_ms()
}

/// A new move (or a rest position) from the island page.
#[tauri::command]
pub fn set_island_motion(motion: Motion) {
    let s = &motion.segment;
    let sane = [s.t0, s.omega, s.from.w, s.from.h, s.to.w, s.to.h, s.velocity.w, s.velocity.h, motion.cx, motion.top]
        .iter()
        .all(|v| v.is_finite())
        && s.omega > 0.0;
    if !sane {
        return;
    }
    *MOTION.lock().unwrap() = Some(motion);
    crate::backdrop::poke();
}

// ---------- measurements (docs/contracts/metrics.md §3) ----------

#[derive(Default)]
struct GlassFrames {
    /// When the previous frame of the current run was drawn.
    last: Option<f64>,
    deltas: Vec<f64>,
    mode: &'static str,
    drawn: Rect,
}

static GLASS: Mutex<GlassFrames> = Mutex::new(GlassFrames { last: None, deltas: Vec::new(), mode: "off", drawn: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0, r: 0.0 } });
const SAMPLE_LIMIT: usize = 20_000;

/// The glass drew a frame (backdrop.rs). `in_run` = part of a continuous move.
pub fn glass_drew(t: f64, in_run: bool, mode: &'static str, drawn: Rect) {
    let mut g = GLASS.lock().unwrap();
    if in_run {
        if let Some(last) = g.last {
            if g.deltas.len() < SAMPLE_LIMIT {
                g.deltas.push(t - last);
            }
        }
        g.last = Some(t);
    } else {
        g.last = None;
    }
    g.mode = mode;
    g.drawn = drawn;
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStats {
    pub budget_ms: f64,
    pub count: usize,
    pub dropped: usize,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub duration_ms: f64,
}

/// Frame statistics from intervals: the budget is the median snapped to a real refresh rate.
pub fn frame_stats(deltas: &[f64]) -> FrameStats {
    let mut sorted = deltas.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| sorted.get(((q * sorted.len() as f64) as usize).min(sorted.len().saturating_sub(1))).copied().unwrap_or(0.0);
    let mid = at(0.5);
    let budget = [60.0, 72.0, 75.0, 90.0, 100.0, 120.0, 144.0, 165.0, 240.0]
        .iter()
        .map(|hz| 1000.0 / hz)
        .min_by(|a: &f64, b: &f64| (a - mid).abs().total_cmp(&(b - mid).abs()))
        .unwrap_or(1000.0 / 60.0);
    let round = |v: f64| (v * 100.0).round() / 100.0;
    FrameStats {
        budget_ms: round(budget),
        count: sorted.len(),
        dropped: sorted.iter().filter(|d| **d > budget * 1.5).count(),
        p50: round(mid),
        p95: round(at(0.95)),
        max: round(sorted.last().copied().unwrap_or(0.0)),
        duration_ms: round(sorted.iter().sum()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlassMetrics {
    pub mode: &'static str,
    pub loop_running: bool,
    pub frames: FrameStats,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeMetrics {
    pub version: u32,
    pub at: f64,
    /// What the glass last drew, and what hover is tested against now (CSS px in the island window).
    pub shape: Rect,
    pub hit: Rect,
    pub glass: GlassMetrics,
}

/// Debug builds only: what the native side is doing, for the dev overlay and the test harness.
#[tauri::command]
pub fn island_metrics(reset: Option<bool>) -> Option<NativeMetrics> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let now = now_ms();
    let mut g = GLASS.lock().unwrap();
    let out = NativeMetrics {
        version: 1,
        at: now,
        shape: g.drawn,
        hit: hit_rect(now, 16.0).unwrap_or_default(),
        glass: GlassMetrics { mode: g.mode, loop_running: moving(now), frames: frame_stats(&g.deltas) },
    };
    if reset == Some(true) {
        g.deltas.clear();
    }
    Some(out)
}

#[cfg(test)]
pub(crate) fn set_for_test(m: Option<Motion>) {
    *MOTION.lock().unwrap() = m;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two tests set the one shared motion; they must not overlap.
    static SHARED: Mutex<()> = Mutex::new(());

    fn close(a: f64, b: f64, eps: f64, what: &str) {
        assert!((a - b).abs() <= eps, "{what}: {a} vs {b}");
    }

    /// A move from the idle pill towards the status width, retargeted to the
    /// panel at 40 ms. The numbers are what spring.ts gives for this segment:
    /// the page and Rust must agree, or the glass would not follow the shape.
    fn panel_segment() -> Segment {
        Segment {
            t0: 40.0,
            from: Shape { w: 177.2602929434645, h: 40.0, r: 20.0 },
            velocity: Shape { w: 3.3830133881978535, h: 0.0, r: 0.0 },
            to: Shape { w: 720.0, h: 440.0, r: 32.0 },
            omega: 0.028,
        }
    }

    #[test]
    fn matches_the_page_spring() {
        // (time, width, height, radius, width velocity) from spring.ts
        let rows = [
            (40.0, 177.260293, 40.0, 20.0, 3.383013),
            (56.67, 256.201760, 72.118663, 20.963560, 5.578801),
            (100.0, 486.741486, 240.207098, 26.006213, 4.329472),
            (250.0, 711.549641, 432.308751, 31.769263, 0.203593),
            (900.0, 720.0, 440.0, 32.0, 0.0),
        ];
        let seg = panel_segment();
        for (t, w, h, r, vw) in rows {
            let (p, v) = evaluate(&seg, t);
            close(p.w, w, 1e-5, "width");
            close(p.h, h, 1e-5, "height");
            close(p.r, r, 1e-5, "radius");
            close(v.w, vw, 1e-5, "width velocity");
        }
        assert!(!at_rest(&seg, 250.0));
        assert!(at_rest(&seg, 900.0));
    }

    #[test]
    fn glass_stays_inside_and_hover_stays_outside_the_moving_shape() {
        let _guard = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        let m = Motion { segment: panel_segment(), cx: 416.0, top: 8.0, hover_min: 128.0 };
        set_for_test(Some(m));
        for t in [45.0, 60.0, 100.0, 180.0, 250.0] {
            let exact = evaluate(&m.segment, t).0;
            let glass = glass_rect(t, 16.67).unwrap();
            let hit = hit_rect(t, 16.0).unwrap();
            assert!(glass.w <= exact.w && glass.h <= exact.h, "glass bleeds at {t}");
            assert!(hit.w >= exact.w && hit.h >= exact.h, "hover misses the edge at {t}");
            // Both stay centred on the same line, hanging from the same top.
            close(glass.x + glass.w / 2.0, 416.0, 1e-9, "glass centre");
            close(hit.x + hit.w / 2.0, 416.0, 1e-9, "hover centre");
            assert_eq!((glass.y, hit.y), (8.0, 8.0));
        }
        // At rest: exactly the target, and no loop.
        let rest = glass_rect(5000.0, 16.67).unwrap();
        assert_eq!((rest.w, rest.h, rest.r), (720.0, 440.0, 32.0));
        assert!(!moving(5000.0));
        assert!(moving(100.0));
        set_for_test(None);
        assert!(glass_rect(0.0, 16.67).is_none() && !moving(0.0));
    }

    #[test]
    fn hover_area_never_narrower_than_its_minimum() {
        let _guard = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        let idle = Shape { w: 72.0, h: 40.0, r: 20.0 };
        let seg = Segment { t0: 0.0, from: idle, velocity: Shape::default(), to: idle, omega: 0.02 };
        set_for_test(Some(Motion { segment: seg, cx: 416.0, top: 8.0, hover_min: 128.0 }));
        let hit = hit_rect(10.0, 16.0).unwrap();
        assert_eq!((hit.x, hit.w, hit.h), (352.0, 128.0, 40.0));
        // The glass is the real 72 px pill, not the hover area.
        assert_eq!(glass_rect(10.0, 16.67).unwrap().w, 72.0);
        set_for_test(None);
    }

    #[test]
    fn frame_stats_count_dropped_frames_against_the_real_refresh_rate() {
        let mut deltas = vec![6.94; 100];
        deltas[10] = 13.9;
        deltas[50] = 27.8;
        let s = frame_stats(&deltas);
        assert_eq!((s.budget_ms, s.count, s.dropped), (6.94, 100, 2));
        assert_eq!(s.max, 27.8);
    }
}
