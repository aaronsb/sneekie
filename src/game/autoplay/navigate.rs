//! Time-aware navigation for the moving-hazard levels.
//!
//! The reactive greedy chain can't anticipate a hazard's *future* position, so it
//! chases a moving wall-gap it can't reach in time, or steps into an arrow's path.
//! Every moving hazard here is deterministic — one cell per snake step — so it can
//! be forecast and routed around in time:
//!
//! - **Arrows** (`↑`=24, `→`=26, `←`=27) advance by a fixed velocity per step and
//!   are *lethal*: entering an arrow cell, or having an arrow step onto the head,
//!   is death. Forecast each by its glyph's velocity.
//! - **Wall gaps** crawl down one row per step. The opening pattern simply shifts
//!   down, so a wall cell is passable at step `s` iff the cell `s` rows above it is
//!   open now (wrapping the 4..20 wall span).
//!
//! A breadth-first search over `(cell, step)` then finds the shortest *timed* path
//! to the nearest heart that's collision-free at every step, and returns its first
//! move. It replans every tick (the hazards have moved), so it commits one move at
//! a time. If no timed path to food exists within the horizon it takes the safest
//! step toward the nearest heart; if even that fails, it returns `None` and the
//! greedy chain takes over.

use std::collections::VecDeque;

use super::{reverse, DIRS};

/// How many steps ahead the forecast/search looks. Past this, arrows may have
/// wrapped and the linear forecast drifts, so keep it modest.
const NAV_HORIZON: usize = 14;

/// The moving hazard a level runs (mirrors `run_enemy`'s mapping).
enum Hazard {
    None,
    Arrows,
    Gaps,
}

impl crate::game::Game {
    fn nav_hazard(&self) -> Hazard {
        match (self.level - 1).rem_euclid(16) {
            5 | 6 | 13 | 14 => Hazard::Arrows,
            4 | 7 | 12 | 15 => Hazard::Gaps,
            _ => Hazard::None,
        }
    }

    /// Pick a move on a moving-hazard level by forecasting the hazards and
    /// routing a timed, collision-free path to the nearest heart.
    pub(super) fn navigate(&self) -> Option<u32> {
        match self.nav_hazard() {
            Hazard::Arrows => self.nav_arrows(),
            Hazard::Gaps => self.nav_gaps(),
            Hazard::None => None,
        }
    }

    /// A cell navigation treats as solid: anything that isn't floor, food, or a
    /// (transient) arrow. Arrows are handled as timed lethality, so a cell merely
    /// holding one right now isn't a wall. The snake's own body reads as solid
    /// (a static-body approximation — conservative over a short horizon).
    fn nav_wall(&self, off: i32) -> bool {
        !matches!(self.peek(off), 32 | 3 | 5 | 24 | 26 | 27)
    }

    fn nav_arrows(&self) -> Option<u32> {
        let mut arrows: Vec<(i32, i32)> = Vec::new();
        for off in (0..4000i32).step_by(2) {
            match self.peek(off) {
                24 => arrows.push((off, -160)), // up
                26 => arrows.push((off, 2)),    // right
                27 => arrows.push((off, -2)),   // left
                _ => {}
            }
        }
        // A cell is lethal at snake-step s if an arrow is there when the head
        // arrives (after s-1 enemy updates) or steps onto it (the s-th update).
        let lethal = |n: i32, s: usize| {
            let s = s as i32;
            arrows
                .iter()
                .any(|&(p, v)| p + v * (s - 1) == n || p + v * s == n)
        };
        let enterable = |n: i32, s: usize| !self.nav_wall(n) && !lethal(n, s);
        self.nav_plan(&enterable)
    }

    fn nav_gaps(&self) -> Option<u32> {
        // Gap walls sit at columns 8,16,..,72; their openings crawl down one row
        // per step, so cell (r,c) is open at step s iff the cell s rows up is open
        // now (wrapping the 17-row wall span, rows 4..=20).
        let open_now = |c1: i32, r1: i32| {
            let off = (r1 - 1) * 160 + (c1 - 1) * 2;
            matches!(self.peek(off), 32 | 3 | 5)
        };
        let enterable = |n: i32, s: usize| {
            let r1 = n / 160 + 1;
            let c1 = (n % 160) / 2 + 1;
            if c1 % 8 == 0 && (8..=72).contains(&c1) {
                let span = 17;
                let rr = 4 + (((r1 - 4 - s as i32) % span + span) % span);
                open_now(c1, rr)
            } else {
                !self.nav_wall(n)
            }
        };
        self.nav_plan(&enterable)
    }

    /// Time-aware BFS: shortest timed path to a heart, returning its first move;
    /// falls back to the safest step toward the nearest heart, then `None`.
    fn nav_plan<F: Fn(i32, usize) -> bool>(&self, enterable: &F) -> Option<u32> {
        let head = self.t[self.btel as usize];
        let rev = reverse(self.e);
        let span = 4000 * (NAV_HORIZON + 1);
        let mut seen = vec![false; span];
        let at = |c: i32, t: usize| t * 4000 + c as usize;
        let mut q: VecDeque<(i32, usize, u32)> = VecDeque::new();
        for (sc, d) in DIRS {
            if sc == rev {
                continue;
            }
            let n = head + d;
            if n >= 0 && (n as usize) < 4000 && enterable(n, 1) && !seen[at(n, 1)] {
                seen[at(n, 1)] = true;
                q.push_back((n, 1, sc));
            }
        }
        while let Some((c, t, fm)) = q.pop_front() {
            if self.is_food(c) {
                return Some(fm);
            }
            if t >= NAV_HORIZON {
                continue;
            }
            for (_sc, d) in DIRS {
                let n = c + d;
                if n >= 0 && (n as usize) < 4000 && enterable(n, t + 1) && !seen[at(n, t + 1)] {
                    seen[at(n, t + 1)] = true;
                    q.push_back((n, t + 1, fm));
                }
            }
        }
        self.nav_safe_step(enterable)
    }

    /// No timed path to food within the horizon: step toward the nearest heart
    /// along whichever immediate move is collision-safe next tick.
    fn nav_safe_step<F: Fn(i32, usize) -> bool>(&self, enterable: &F) -> Option<u32> {
        let head = self.t[self.btel as usize];
        let rev = reverse(self.e);
        let target = self.nearest_heart(head);
        let mut best: Option<u32> = None;
        let mut best_key = i32::MAX;
        for (sc, d) in DIRS {
            if sc == rev {
                continue;
            }
            let n = head + d;
            if n < 0 || (n as usize) >= 4000 || !enterable(n, 1) {
                continue;
            }
            let key = target.map_or(0, |t| manhattan(n, t));
            if key < best_key {
                best_key = key;
                best = Some(sc);
            }
        }
        best
    }

    /// Nearest heart/club to `from` by Manhattan distance (None if cleared).
    fn nearest_heart(&self, from: i32) -> Option<i32> {
        let mut best: Option<i32> = None;
        let mut best_d = i32::MAX;
        for off in (0..4000i32).step_by(2) {
            if matches!(self.peek(off), 3 | 5) {
                let d = manhattan(from, off);
                if d < best_d {
                    best_d = d;
                    best = Some(off);
                }
            }
        }
        best
    }
}

fn manhattan(a: i32, b: i32) -> i32 {
    let (ar, ac) = (a / 160, (a % 160) / 2);
    let (br, bc) = (b / 160, (b % 160) / 2);
    (ar - br).abs() + (ac - bc).abs()
}
