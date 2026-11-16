// What the island shows, decided in one place.
//
// Every source (the agent, the focus timer, the calendar, music, the person
// using the PC) only says what it has to offer: a CLAIM. The arbiter picks
// which claim gets the island, and remembers enough to do it calmly:
//
// - Preemption: "needs you" and errors take the island at once. So does the
//   focus timer (the person started it). Everything else takes it only when
//   the state on screen has been up for a moment (MIN_DWELL_MS).
// - Coalescing: the same kind of thing arriving again (a second task finishing
//   while the first is still announced) joins the claim already there and is
//   counted; it is not a new arrival.
// - Rate limits: a source may take the island away from another state only so
//   often (RATE_MS), so an agent that flickers between busy and idle cannot
//   make the pill flicker with it.
// - Hold-back: during a focus session with Do not disturb, an error waits
//   behind the timer (unless Settings says to show it) and is shown when the
//   session ends.
// - Clean return: claims stay while their source keeps offering them, so when
//   the state on screen ends, the best claim still waiting comes back, without
//   delay or rate limit.
//
// The island has two regions. The PILL shows one of the pill states. A SURFACE
// (a peek or the open panel) may cover it; surfaces follow a fixed order
// because they are either the person's own doing (panel) or cannot wait
// (a permission request).
//
// The core (`Arbiter::step`) is pure: claims and a time go in, a decision
// comes out. No timers, no threads: when a claim is waiting on a clock, the
// decision says when to ask again (`recheck_in`), and the page does.
// Tests at the bottom are tables: one row per transition.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// The state on screen is left alone this long before a non-urgent claim may replace it.
pub const MIN_DWELL_MS: f64 = 400.0;
/// A claim re-arriving with a new identity inside this window is counted with the one before.
pub const COALESCE_MS: f64 = 1500.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Key {
    // Surfaces, in their fixed order.
    #[serde(rename = "peek:permission")]
    PeekPermission,
    #[serde(rename = "peek:question")]
    PeekQuestion,
    #[serde(rename = "peek:ask")]
    PeekAsk,
    #[serde(rename = "panel")]
    Panel,
    #[serde(rename = "peek:digest")]
    PeekDigest,
    #[serde(rename = "peek:finished")]
    PeekFinished,
    // Pill states.
    #[serde(rename = "needs-you")]
    NeedsYou,
    #[serde(rename = "error")]
    Error,
    #[serde(rename = "focus")]
    Focus,
    #[serde(rename = "event")]
    Event,
    #[serde(rename = "busy")]
    Busy,
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "music")]
    Music,
    #[serde(rename = "idle")]
    Idle,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Agent,
    Focus,
    Calendar,
    Music,
    User,
    Island,
}

const SURFACES: [Key; 6] = [Key::PeekPermission, Key::PeekQuestion, Key::PeekAsk, Key::Panel, Key::PeekDigest, Key::PeekFinished];

impl Key {
    pub fn is_surface(self) -> bool {
        SURFACES.contains(&self)
    }

    pub fn source(self) -> Source {
        match self {
            Key::PeekPermission | Key::PeekQuestion | Key::PeekAsk | Key::PeekFinished | Key::NeedsYou | Key::Error | Key::Busy | Key::Done => Source::Agent,
            Key::PeekDigest | Key::Focus => Source::Focus,
            Key::Event => Source::Calendar,
            Key::Music => Source::Music,
            Key::Panel => Source::User,
            Key::Idle => Source::Island,
        }
    }
}

/// How often a source may take the pill away from another source's state, ms. 0 = no limit.
pub fn rate_ms(source: Source) -> f64 {
    match source {
        Source::Agent => 1000.0,
        Source::Music => 1000.0,
        Source::Calendar => 3000.0,
        Source::Focus | Source::User | Source::Island => 0.0,
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Claim {
    pub key: Key,
    /// What exactly is claimed (a session id, a track id). A new token on a
    /// claim already present is the same kind of thing arriving again.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    /// Settings → Spotify: music gets the pill ahead of a busy or finished agent.
    #[serde(default)]
    pub music_first: bool,
    /// A focus session is on with Do not disturb.
    #[serde(default)]
    pub focus_dnd: bool,
    /// Settings → Focus timer: errors are shown during focus instead of held.
    #[serde(default)]
    pub errors_in_focus: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Input {
    /// Counts up with every post, so the page can drop an answer that arrives late.
    #[serde(default)]
    pub seq: u64,
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub policy: Policy,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Outranked by what is showing.
    Waiting,
    /// Held back by the focus session.
    Held,
    /// Would win, but the state on screen has not been up long enough.
    Dwell,
    /// Would win, but its source took the pill too recently.
    RateLimited,
    /// The pill state under a peek or the open panel.
    Covered,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub key: Key,
    pub source: Source,
    pub status: Status,
    pub reason: String,
    /// How many arrivals this claim stands for.
    pub count: u32,
    pub waited_ms: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cause {
    /// An urgent claim took over from a state that was still on.
    Preempted,
    /// A higher claim took over, after the dwell and within its rate limit.
    Replaced,
    /// The state on screen ended and a claim that had been waiting came back.
    Returned,
    /// The state on screen ended and something new took its place.
    Followed,
}

#[derive(Clone, Debug, Serialize)]
pub struct Transition {
    pub from: Key,
    pub to: Key,
    pub cause: Cause,
    pub at: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub seq: u64,
    /// What the island shows: the surface if there is one, else the pill state.
    pub state: Key,
    pub pill: Key,
    pub surface: Option<Key>,
    pub reason: String,
    /// The pill states on offer, best first. The page shows the first one it
    /// still has, so a state that ends is replaced without waiting for an answer.
    pub order: Vec<Key>,
    pub queue: Vec<QueueItem>,
    /// The last change of `state`.
    pub last: Option<Transition>,
    /// Ask again after this many ms (a dwell or rate limit will have run out).
    pub recheck_in: Option<f64>,
}

struct Presence {
    since: f64,
    token: Option<String>,
    count: u32,
    arrived: f64,
}

struct Ranked {
    key: Key,
    held: bool,
    why: &'static str,
}

pub struct Arbiter {
    present: HashMap<Key, Presence>,
    pill: Key,
    pill_since: f64,
    state: Key,
    last_takeover: HashMap<Source, f64>,
    last: Option<Transition>,
}

impl Default for Arbiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Arbiter {
    pub fn new() -> Self {
        Self { present: HashMap::new(), pill: Key::Idle, pill_since: f64::NEG_INFINITY, state: Key::Idle, last_takeover: HashMap::new(), last: None }
    }

    /// The pill states on offer in rank order, before any timing rule.
    fn rank(&self, policy: Policy) -> Vec<Ranked> {
        let has = |k: Key| self.present.contains_key(&k);
        let mut out = Vec::new();
        let mut push = |key: Key, held: bool, why: &'static str| {
            if has(key) {
                out.push(Ranked { key, held, why });
            }
        };
        let error_held = has(Key::Focus) && policy.focus_dnd && !policy.errors_in_focus;
        push(Key::NeedsYou, false, "needs you: always first");
        if !error_held {
            push(Key::Error, false, "an error: first, after needs you");
        }
        push(Key::Focus, false, "focus timer on: beats music and agent status");
        if error_held {
            push(Key::Error, true, "held back during focus");
        }
        // An event about to start beats music, idle and a finished task, but
        // not an agent at work.
        let event_blocked = has(Key::Busy);
        if policy.music_first {
            if !event_blocked {
                push(Key::Event, false, "an event starts soon and nothing urgent is on");
            }
            push(Key::Music, false, "music is given the pill (Settings → Spotify)");
            push(Key::Busy, false, "agent status");
            if event_blocked {
                push(Key::Event, false, "waits while the agent is working");
            }
            push(Key::Done, false, "agent status");
        } else {
            push(Key::Busy, false, "agent status");
            push(Key::Event, false, "an event starts soon and nothing urgent is on");
            push(Key::Done, false, "agent status");
            push(Key::Music, false, "music playing, nothing more urgent");
        }
        out.push(Ranked { key: Key::Idle, held: false, why: "nothing to show" });
        out
    }

    pub fn step(&mut self, input: &Input, now: f64) -> Decision {
        // 1. Who is claiming. A claim already present keeps its place; a new
        //    token on it is counted with it (coalescing).
        let mut seen: Vec<Key> = Vec::new();
        for c in &input.claims {
            if c.key == Key::Idle || seen.contains(&c.key) {
                continue;
            }
            seen.push(c.key);
            match self.present.get_mut(&c.key) {
                Some(p) => {
                    if p.token != c.token {
                        p.count = if now - p.arrived <= COALESCE_MS { p.count + 1 } else { 1 };
                        p.arrived = now;
                        p.token = c.token.clone();
                    }
                }
                None => {
                    self.present.insert(c.key, Presence { since: now, token: c.token.clone(), count: 1, arrived: now });
                }
            }
        }
        self.present.retain(|k, _| seen.contains(k));

        // 2. The pill.
        let ranked = self.rank(input.policy);
        let urgent = |r: &Ranked| !r.held && matches!(r.key, Key::NeedsYou | Key::Error | Key::Focus);
        let shown_alive = self.pill == Key::Idle || self.present.contains_key(&self.pill);
        let shown_at = ranked.iter().position(|r| r.key == self.pill);
        let mut blocked: HashMap<Key, (Status, f64)> = HashMap::new();
        let mut winner = self.pill;
        let mut cause = None;
        if !shown_alive {
            // What was showing has ended: the best claim takes its place, no questions asked.
            let best = &ranked[0];
            winner = best.key;
            let waited = self.present.get(&best.key).map(|p| p.since < now).unwrap_or(best.key == Key::Idle);
            cause = Some(if waited { Cause::Returned } else { Cause::Followed });
        } else {
            let above = shown_at.unwrap_or(ranked.len());
            for r in &ranked[..above] {
                if urgent(r) {
                    winner = r.key;
                    cause = Some(Cause::Preempted);
                    break;
                }
                // Nothing is lost by replacing idle at once.
                let dwell_left = if self.pill == Key::Idle { 0.0 } else { self.pill_since + MIN_DWELL_MS - now };
                let source = r.key.source();
                let rate_left = if source == self.pill.source() {
                    0.0 // the same source moving on (busy to done) is not a takeover
                } else {
                    self.last_takeover.get(&source).map(|t| t + rate_ms(source) - now).unwrap_or(0.0)
                };
                if rate_left > 0.0 && rate_left >= dwell_left {
                    blocked.insert(r.key, (Status::RateLimited, rate_left));
                } else if dwell_left > 0.0 {
                    blocked.insert(r.key, (Status::Dwell, dwell_left));
                } else {
                    winner = r.key;
                    cause = Some(Cause::Replaced);
                    break;
                }
            }
        }
        if winner != self.pill {
            if matches!(cause, Some(Cause::Preempted | Cause::Replaced)) && winner.source() != self.pill.source() {
                self.last_takeover.insert(winner.source(), now);
            }
            self.pill = winner;
            self.pill_since = now;
        }

        // 3. The surface: fixed order.
        let surface = SURFACES.iter().copied().find(|k| self.present.contains_key(k));
        let state = surface.unwrap_or(self.pill);
        if state != self.state {
            let cause = match (surface, self.state.is_surface()) {
                (Some(_), _) => Cause::Preempted,
                (None, true) => Cause::Returned,
                (None, false) => cause.unwrap_or(Cause::Replaced),
            };
            self.last = Some(Transition { from: self.state, to: state, cause, at: now });
            self.state = state;
        }

        // 4. The answer.
        let mut order: Vec<Key> = vec![self.pill];
        order.extend(ranked.iter().map(|r| r.key).filter(|k| *k != self.pill));
        let pill_why = ranked.iter().find(|r| r.key == self.pill).map(|r| r.why).unwrap_or("nothing to show");
        let reason = match surface {
            Some(Key::PeekPermission | Key::PeekQuestion) => "a permission request or question is waiting".to_string(),
            Some(Key::PeekAsk) => "Claude asked a question".to_string(),
            Some(Key::Panel) => "opened by the person using it".to_string(),
            Some(Key::PeekDigest) => "break started: what was held during focus".to_string(),
            Some(Key::PeekFinished) => "a task just finished".to_string(),
            _ => pill_why.to_string(),
        };
        let mut queue = Vec::new();
        let item = |key: Key, status: Status, reason: String, present: &HashMap<Key, Presence>| {
            let p = present.get(&key);
            QueueItem { key, source: key.source(), status, reason, count: p.map(|p| p.count).unwrap_or(1), waited_ms: p.map(|p| now - p.since).unwrap_or(0.0) }
        };
        for k in SURFACES.iter().copied().filter(|k| self.present.contains_key(k) && Some(*k) != surface) {
            queue.push(item(k, Status::Waiting, "behind the surface that is showing".into(), &self.present));
        }
        if surface.is_some() && self.pill != Key::Idle {
            queue.push(item(self.pill, Status::Covered, format!("under the surface: {pill_why}"), &self.present));
        }
        for r in ranked.iter().filter(|r| r.key != self.pill && r.key != Key::Idle) {
            let (status, why) = match blocked.get(&r.key) {
                Some((Status::RateLimited, left)) => (Status::RateLimited, format!("its source took the pill a moment ago: {:.0} ms to go", left)),
                Some((_, left)) => (Status::Dwell, format!("what is showing stays up a moment: {:.0} ms to go", left)),
                None if r.held => (Status::Held, r.why.to_string()),
                None => (Status::Waiting, if r.key == Key::Event && self.present.contains_key(&Key::Busy) { r.why.to_string() } else { format!("behind {}", name(self.pill)) }),
            };
            queue.push(item(r.key, status, why, &self.present));
        }
        let recheck_in = blocked.values().map(|(_, left)| *left).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.min(v))));
        Decision { seq: input.seq, state, pill: self.pill, surface, reason, order, queue, last: self.last.clone(), recheck_in }
    }
}

fn name(key: Key) -> &'static str {
    match key {
        Key::PeekPermission => "the permission request",
        Key::PeekQuestion => "the question",
        Key::PeekAsk => "Claude's question",
        Key::Panel => "the open panel",
        Key::PeekDigest => "the focus digest",
        Key::PeekFinished => "the finished task",
        Key::NeedsYou => "needs you",
        Key::Error => "the error",
        Key::Focus => "the focus timer",
        Key::Event => "the event",
        Key::Busy => "the busy agent",
        Key::Done => "the finished agent",
        Key::Music => "music",
        Key::Idle => "idle",
    }
}

// ---------- the running island ----------

static ARBITER: Mutex<Option<Arbiter>> = Mutex::new(None);

/// What `island-state` carries: `IslandMetrics["state"]` in docs/contracts/metrics.md.
#[derive(Clone, Serialize)]
struct StateEvent {
    id: Key,
    previous: Option<Key>,
    since: f64,
    reason: String,
    queue: Vec<QueueEvent>,
}

#[derive(Clone, Serialize)]
struct QueueEvent {
    id: Key,
    source: Source,
    at: f64,
}

/// The page posts what every source offers; the answer says what to show.
#[tauri::command]
pub fn arbiter_post(app: AppHandle, input: Input) -> Decision {
    let now = crate::shape::now_ms();
    let mut guard = ARBITER.lock().unwrap();
    let arbiter = guard.get_or_insert_with(Arbiter::new);
    let before = arbiter.state;
    let decision = arbiter.step(&input, now);
    if decision.state != before {
        let queue = decision.queue.iter().map(|q| QueueEvent { id: q.key, source: q.source, at: now - q.waited_ms }).collect();
        let _ = app.emit("island-state", StateEvent { id: decision.state, previous: Some(before), since: now, reason: decision.reason.clone(), queue });
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;
    use Key::*;

    const PLAIN: Policy = Policy { music_first: false, focus_dnd: false, errors_in_focus: false };
    const MUSIC_FIRST: Policy = Policy { music_first: true, focus_dnd: false, errors_in_focus: false };
    const DND: Policy = Policy { music_first: false, focus_dnd: true, errors_in_focus: false };
    const DND_SHOW_ERRORS: Policy = Policy { music_first: false, focus_dnd: true, errors_in_focus: true };

    fn input(keys: &[Key], policy: Policy) -> Input {
        Input { seq: 0, claims: keys.iter().map(|k| Claim { key: *k, token: None }).collect(), policy }
    }

    /// A fresh island given these claims long enough for every clock to run out.
    fn settled(keys: &[Key], policy: Policy) -> Decision {
        let mut a = Arbiter::new();
        a.step(&input(keys, policy), 0.0);
        a.step(&input(keys, policy), 60_000.0)
    }

    #[test]
    fn todays_priority_is_reproduced() {
        // [claims, policy, pill] — every row was checked against the page's
        // old rule (pillKind in App.svelte before the arbiter).
        let rows: &[(&[Key], Policy, Key)] = &[
            (&[], PLAIN, Idle),
            (&[Music], PLAIN, Music),
            (&[Busy], PLAIN, Busy),
            (&[Done], PLAIN, Done),
            (&[Event], PLAIN, Event),
            (&[Focus], PLAIN, Focus),
            (&[Error], PLAIN, Error),
            (&[NeedsYou], PLAIN, NeedsYou),
            // Agent status beats music unless Settings gives music the pill.
            (&[Busy, Music], PLAIN, Busy),
            (&[Done, Music], PLAIN, Done),
            (&[Busy, Music], MUSIC_FIRST, Music),
            (&[Done, Music], MUSIC_FIRST, Music),
            // Needs you and errors always beat music.
            (&[NeedsYou, Music], MUSIC_FIRST, NeedsYou),
            (&[Error, Music], MUSIC_FIRST, Error),
            // An event beats music, idle and a finished task; a working agent beats it.
            (&[Event, Music], PLAIN, Event),
            (&[Event, Done], PLAIN, Event),
            (&[Event, Busy], PLAIN, Busy),
            (&[Event, Music], MUSIC_FIRST, Event),
            (&[Event, Done, Music], MUSIC_FIRST, Event),
            (&[Event, Busy], MUSIC_FIRST, Busy),
            (&[Event, Busy, Music], MUSIC_FIRST, Music),
            (&[Event, Error], PLAIN, Error),
            (&[Event, NeedsYou], PLAIN, NeedsYou),
            // The focus timer beats everything but needs you, and errors unless focus holds them.
            (&[Focus, Music], PLAIN, Focus),
            (&[Focus, Busy], PLAIN, Focus),
            (&[Focus, Done], PLAIN, Focus),
            (&[Focus, Event], PLAIN, Focus),
            (&[Focus, Event, Busy, Music], MUSIC_FIRST, Focus),
            (&[Focus, NeedsYou], PLAIN, NeedsYou),
            (&[Focus, NeedsYou], DND, NeedsYou),
            (&[Focus, Error], PLAIN, Error),           // a break, or Do not disturb off
            (&[Focus, Error], DND, Focus),             // held
            (&[Focus, Error], DND_SHOW_ERRORS, Error), // Settings: show errors during focus
            (&[Error, Music], DND, Error),             // no timer claim: nothing holds it
            (&[NeedsYou, Error, Focus, Event, Busy, Music], PLAIN, NeedsYou),
        ];
        for (claims, policy, want) in rows {
            let d = settled(claims, *policy);
            assert_eq!(d.pill, *want, "claims {claims:?}, {policy:?}");
            assert_eq!(d.order[0], *want);
            assert_eq!(d.recheck_in, None, "nothing may be left waiting on a clock: {claims:?}");
        }
    }

    #[test]
    fn surfaces_follow_a_fixed_order_and_cover_the_pill() {
        // [claims, surface, state]
        let rows: &[(&[Key], Option<Key>, Key)] = &[
            (&[Music], None, Music),
            (&[Panel, Music], Some(Panel), Panel),
            (&[PeekFinished, Music], Some(PeekFinished), PeekFinished),
            (&[PeekDigest, PeekFinished], Some(PeekDigest), PeekDigest),
            (&[Panel, PeekFinished, PeekDigest], Some(Panel), Panel),
            (&[PeekAsk, Panel], Some(PeekAsk), PeekAsk),
            (&[PeekPermission, PeekAsk, Panel], Some(PeekPermission), PeekPermission),
            (&[PeekQuestion, Panel, NeedsYou], Some(PeekQuestion), PeekQuestion),
        ];
        for (claims, surface, state) in rows {
            let d = settled(claims, PLAIN);
            assert_eq!(d.surface, *surface, "{claims:?}");
            assert_eq!(d.state, *state, "{claims:?}");
        }
        // The pill under a surface is still decided, and listed as covered.
        let d = settled(&[Panel, Busy, Music], PLAIN);
        assert_eq!(d.pill, Busy);
        assert!(d.queue.iter().any(|q| q.key == Busy && q.status == Status::Covered));
        assert!(d.queue.iter().any(|q| q.key == Music && q.status == Status::Waiting));
    }

    /// One step of a script: at `at` ms these claims are posted; the pill must be `pill`.
    struct Step {
        at: f64,
        claims: &'static [Key],
        pill: Key,
        cause: Option<Cause>,
        recheck: Option<f64>,
        note: &'static str,
    }

    fn run(policy: Policy, steps: &[Step]) {
        let mut a = Arbiter::new();
        for s in steps {
            let before = a.state;
            let d = a.step(&input(s.claims, policy), s.at);
            assert_eq!(d.pill, s.pill, "{} (at {} ms)", s.note, s.at);
            assert_eq!(d.recheck_in, s.recheck, "{} (at {} ms): recheck", s.note, s.at);
            if d.state != before {
                assert_eq!(d.last.as_ref().map(|t| t.cause), s.cause, "{} (at {} ms): cause", s.note, s.at);
                assert_eq!(d.last.as_ref().map(|t| (t.from, t.to)), Some((before, d.state)));
            } else {
                assert_eq!(s.cause, None, "{} (at {} ms): no change expected", s.note, s.at);
            }
        }
    }

    #[test]
    fn urgent_claims_preempt_at_once_and_the_previous_state_returns() {
        run(PLAIN, &[
            Step { at: 0.0, claims: &[Music], pill: Music, cause: Some(Cause::Replaced), recheck: None, note: "music replaces idle at once" },
            Step { at: 50.0, claims: &[Music, NeedsYou], pill: NeedsYou, cause: Some(Cause::Preempted), recheck: None, note: "needs you does not wait for the dwell" },
            Step { at: 60.0, claims: &[Music, NeedsYou, Error], pill: NeedsYou, cause: None, recheck: None, note: "an error does not outrank needs you" },
            Step { at: 70.0, claims: &[Music, Error], pill: Error, cause: Some(Cause::Returned), recheck: None, note: "needs you answered: the error was waiting" },
            Step { at: 80.0, claims: &[Music], pill: Music, cause: Some(Cause::Returned), recheck: None, note: "error cleared: back to music, no delay" },
            Step { at: 90.0, claims: &[], pill: Idle, cause: Some(Cause::Returned), recheck: None, note: "music stopped" },
        ]);
    }

    #[test]
    fn focus_starts_at_once_holds_errors_and_releases_them() {
        run(DND, &[
            Step { at: 0.0, claims: &[Music], pill: Music, cause: Some(Cause::Replaced), recheck: None, note: "music" },
            Step { at: 100.0, claims: &[Music, Focus], pill: Focus, cause: Some(Cause::Preempted), recheck: None, note: "the person started a timer: no dwell" },
            Step { at: 5_000.0, claims: &[Music, Focus, Error], pill: Focus, cause: None, recheck: None, note: "error held behind focus" },
            Step { at: 6_000.0, claims: &[Music, Focus, Error, NeedsYou], pill: NeedsYou, cause: Some(Cause::Preempted), recheck: None, note: "needs you is never held" },
            Step { at: 7_000.0, claims: &[Music, Focus, Error], pill: Focus, cause: Some(Cause::Returned), recheck: None, note: "back to the timer, not to the held error" },
            Step { at: 9_000.0, claims: &[Music, Error], pill: Error, cause: Some(Cause::Returned), recheck: None, note: "focus over: the held error is shown" },
            Step { at: 9_500.0, claims: &[Music], pill: Music, cause: Some(Cause::Returned), recheck: None, note: "then music again" },
        ]);
        // The held error is reported as held, with its reason.
        let d = settled(&[Focus, Error, Music], DND);
        let held = d.queue.iter().find(|q| q.key == Error).unwrap();
        assert_eq!((held.status, held.reason.as_str()), (Status::Held, "held back during focus"));
    }

    #[test]
    fn a_new_state_waits_for_the_one_on_screen_to_have_its_moment() {
        run(PLAIN, &[
            Step { at: 0.0, claims: &[Music], pill: Music, cause: Some(Cause::Replaced), recheck: None, note: "a song starts" },
            Step { at: 100.0, claims: &[Music, Busy], pill: Music, cause: None, recheck: Some(300.0), note: "the agent starts 100 ms later: music keeps its 400 ms" },
            Step { at: 399.0, claims: &[Music, Busy], pill: Music, cause: None, recheck: Some(1.0), note: "still inside the dwell" },
            Step { at: 400.0, claims: &[Music, Busy], pill: Busy, cause: Some(Cause::Replaced), recheck: None, note: "dwell over: busy takes the pill" },
        ]);
        // Idle has nothing to protect.
        run(PLAIN, &[Step { at: 0.0, claims: &[Busy], pill: Busy, cause: Some(Cause::Replaced), recheck: None, note: "idle is replaced at once" }]);
    }

    #[test]
    fn a_flickering_source_is_rate_limited() {
        run(PLAIN, &[
            Step { at: 0.0, claims: &[Music], pill: Music, cause: Some(Cause::Replaced), recheck: None, note: "music" },
            Step { at: 5_000.0, claims: &[Music, Busy], pill: Busy, cause: Some(Cause::Replaced), recheck: None, note: "agent takes the pill" },
            Step { at: 5_200.0, claims: &[Music], pill: Music, cause: Some(Cause::Returned), recheck: None, note: "agent idle again: music returns at once" },
            Step { at: 5_700.0, claims: &[Music, Busy], pill: Music, cause: None, recheck: Some(300.0), note: "busy again 700 ms after its last takeover: wait" },
            Step { at: 5_900.0, claims: &[Music], pill: Music, cause: None, recheck: None, note: "and gone again: the pill never moved" },
            Step { at: 5_950.0, claims: &[Music, Busy], pill: Music, cause: None, recheck: Some(50.0), note: "third flicker, still limited" },
            Step { at: 6_000.0, claims: &[Music, Busy], pill: Busy, cause: Some(Cause::Replaced), recheck: None, note: "a second after the last takeover it may again" },
        ]);
        // Urgent claims are never limited.
        run(PLAIN, &[
            Step { at: 0.0, claims: &[Music], pill: Music, cause: Some(Cause::Replaced), recheck: None, note: "music" },
            Step { at: 1_000.0, claims: &[Music, Busy], pill: Busy, cause: Some(Cause::Replaced), recheck: None, note: "busy" },
            Step { at: 1_100.0, claims: &[Music], pill: Music, cause: Some(Cause::Returned), recheck: None, note: "idle" },
            Step { at: 1_200.0, claims: &[Music, Error], pill: Error, cause: Some(Cause::Preempted), recheck: None, note: "an error 200 ms after the agent's last takeover still shows at once" },
        ]);
    }

    #[test]
    fn one_source_moving_on_is_not_a_takeover() {
        run(PLAIN, &[
            Step { at: 0.0, claims: &[Music], pill: Music, cause: Some(Cause::Replaced), recheck: None, note: "music" },
            Step { at: 1_000.0, claims: &[Music, Busy], pill: Busy, cause: Some(Cause::Replaced), recheck: None, note: "busy takes the pill" },
            Step { at: 1_300.0, claims: &[Music, Done], pill: Done, cause: Some(Cause::Followed), recheck: None, note: "busy became done 300 ms later: no limit, music does not flash in between" },
            Step { at: 1_400.0, claims: &[Music, Busy], pill: Busy, cause: Some(Cause::Followed), recheck: None, note: "and busy again" },
            Step { at: 1_500.0, claims: &[Music, Busy, Event], pill: Busy, cause: None, recheck: None, note: "an event does not interrupt a working agent" },
        ]);
    }

    #[test]
    fn repeats_are_counted_together() {
        let post = |a: &mut Arbiter, at: f64, token: &str| {
            let input = Input { seq: 0, claims: vec![Claim { key: PeekFinished, token: Some(token.into()) }, Claim { key: Music, token: None }], policy: PLAIN };
            a.step(&input, at)
        };
        let count = |d: &Decision| d.queue.iter().find(|q| q.key == PeekFinished).map(|q| q.count);
        let mut a = Arbiter::new();
        let d = post(&mut a, 0.0, "session-a");
        assert_eq!(d.state, PeekFinished);
        let first = d.last.clone().unwrap().at;
        // [at, token, arrivals counted] — the peek is one state throughout.
        let rows = [(500.0, "session-b", 2), (900.0, "session-b", 2), (1_200.0, "session-c", 3), (9_000.0, "session-d", 1)];
        for (at, token, want) in rows {
            let d = post(&mut a, at, token);
            assert_eq!(d.state, PeekFinished);
            assert_eq!(d.last.as_ref().unwrap().at, first, "no new transition at {at}");
            assert_eq!(a.present[&PeekFinished].count, want, "at {at}");
            assert_eq!(count(&d), None, "the showing state is not in its own queue");
        }
    }

    #[test]
    fn the_order_lets_the_page_fall_back_without_asking() {
        // The page shows the first state in `order` it still has.
        let d = settled(&[NeedsYou, Focus, Busy, Music], PLAIN);
        assert_eq!(d.order, vec![NeedsYou, Focus, Busy, Music, Idle]);
        // While something waits on a clock, what is showing still comes first.
        let mut a = Arbiter::new();
        a.step(&input(&[Music], PLAIN), 0.0);
        let d = a.step(&input(&[Music, Busy], PLAIN), 100.0);
        assert_eq!(d.order, vec![Music, Busy, Idle]);
        assert!(d.queue.iter().any(|q| q.key == Busy && q.status == Status::Dwell));
    }

    #[test]
    fn late_and_duplicate_claims_are_harmless() {
        let mut a = Arbiter::new();
        let twice = Input { seq: 7, claims: vec![Claim { key: Music, token: None }, Claim { key: Music, token: None }, Claim { key: Idle, token: None }], policy: PLAIN };
        let d = a.step(&twice, 0.0);
        assert_eq!((d.seq, d.pill, d.queue.len()), (7, Music, 0));
    }

    #[test]
    fn the_wire_names_match_the_page() {
        let json = serde_json::to_string(&[PeekPermission, NeedsYou, Idle]).unwrap();
        assert_eq!(json, r#"["peek:permission","needs-you","idle"]"#);
        let input: Input = serde_json::from_str(r#"{"seq":3,"claims":[{"key":"peek:ask","token":"s1"},{"key":"music"}],"policy":{"musicFirst":true}}"#).unwrap();
        assert_eq!((input.seq, input.claims.len(), input.policy.music_first, input.policy.focus_dnd), (3, 2, true, false));
    }
}

#[cfg(test)]
mod probes {
    use super::*;

    /// How long one decision takes. Run: cargo test probe_decision_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_decision_cost() {
        let keys = [Key::Panel, Key::PeekFinished, Key::NeedsYou, Key::Error, Key::Focus, Key::Event, Key::Busy, Key::Music];
        let mut a = Arbiter::new();
        let n = 200_000;
        let start = std::time::Instant::now();
        for i in 0..n {
            // A different mix of claims every step, so every branch is taken.
            let claims = keys.iter().enumerate().filter(|(b, _)| (i >> b) & 1 == 1).map(|(_, k)| Claim { key: *k, token: None }).collect();
            let d = a.step(&Input { seq: i as u64, claims, policy: Policy::default() }, i as f64 * 37.0);
            std::hint::black_box(d);
        }
        println!("{n} decisions, {:.2} µs each", start.elapsed().as_secs_f64() * 1e6 / n as f64);
    }
}
