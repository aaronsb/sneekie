//! The bounded forward-search planner for autoplay.
//!
//! Several cores of modern compute against an 8086 GW-BASIC game: this snapshots
//! the live board into a [`Sim`] and runs a budget-capped **beam search** over
//! move sequences, rolling each candidate forward and scoring the resulting
//! state. It returns the move sequence (scan codes) to the best state it found;
//! the bot queues that plan and executes it tick-by-tick, replanning when the
//! plan empties, the board reseeds (a heart was eaten), or the next queued step
//! is no longer legal.
//!
//! Scoring, in priority order: food eaten (dominant), whether the head can still
//! reach its tail (the immortality guarantee), distance to the nearest remaining
//! food (pulls the search toward heart clusters), and open space (a tiebreak).
//!
//! Lookahead only pays off where the world is static, so the caller restricts
//! the planner to the static maze levels — Sneekie+ hunters and the moving-arrow
//! levels fall through to the per-tick greedy chain instead.

use std::collections::{HashSet, VecDeque};

use super::sim::{Outcome, Sim};
use super::DIRS;

/// Hard cap on simulated steps per replan — the "bailout after N simulations".
/// Rarely reached: a typical replan expands only beam_width × depth × 4 nodes.
pub(super) const PLAN_BUDGET: i32 = 3000;
/// How many moves deep the beam looks.
pub(super) const PLAN_DEPTH: usize = 22;
/// Beam width: survivors kept after each depth layer.
pub(super) const BEAM_WIDTH: usize = 6;

struct Node {
    sim: Sim,
    path: Vec<u32>,
    eaten: i32,
}

impl crate::game::Game {
    /// The immutable static-obstacle map (true = impassable) read from VRAM:
    /// everything that is not empty space, food, or the (movable) snake body.
    fn build_blocked(&self) -> Vec<bool> {
        let mut blocked = vec![false; 4000];
        for off in (0..4000).step_by(2) {
            blocked[off] = self.static_blocked(off as i32);
        }
        blocked
    }

    /// Snapshot the live board into a [`Sim`]: the body (tail→head) and the set
    /// of remaining heart/club cells.
    fn build_sim(&self) -> Sim {
        let mut body: VecDeque<i32> = VecDeque::new();
        for i in self.etel..=self.btel {
            body.push_back(self.t[i as usize]);
        }
        let mut food: HashSet<i32> = HashSet::new();
        for off in (0..4000i32).step_by(2) {
            if matches!(self.peek(off), 3 | 5) {
                food.insert(off);
            }
        }
        Sim::new(body, food)
    }

    /// Multi-source BFS from every food cell, ignoring the snake body, giving a
    /// distance-to-nearest-food for any cell in O(1). The body-ignoring
    /// approximation is fine — this only *guides* the beam toward clusters; the
    /// per-node tail-safety check is what actually keeps moves survivable.
    fn food_dist_field(&self, blocked: &[bool]) -> Vec<i32> {
        let mut dist = vec![i32::MAX; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        for off in (0..4000).step_by(2) {
            if matches!(self.peek(off as i32), 3 | 5) {
                dist[off] = 0;
                q.push_back(off as i32);
            }
        }
        while let Some(c) = q.pop_front() {
            let dc = dist[c as usize];
            for (_sc, d) in DIRS {
                let n = c + d;
                if n >= 0
                    && (n as usize) < 4000
                    && !blocked[n as usize]
                    && dist[n as usize] == i32::MAX
                {
                    dist[n as usize] = dc + 1;
                    q.push_back(n);
                }
            }
        }
        dist
    }

    fn score_node(&self, sim: &Sim, blocked: &[bool], dist: &[i32], eaten: i32) -> i64 {
        let mut s = eaten as i64 * 1_000_000;
        if sim.tail_reachable(blocked) {
            s += 100_000;
        }
        let dh = dist[sim.head() as usize];
        let dp = if dh == i32::MAX { 300 } else { dh.min(300) };
        s -= dp as i64 * 300;
        s += sim.open_space(blocked) as i64;
        s
    }

    /// Beam search; returns the move sequence to the best state found, or `None`
    /// if nothing survives the first ply (then the greedy fallback drives).
    pub(super) fn plan_path(&self) -> Option<VecDeque<u32>> {
        let blocked = self.build_blocked();
        let init = self.build_sim();
        if init.food_left() == 0 {
            return None; // level effectively cleared — nothing to route toward
        }
        let dist = self.food_dist_field(&blocked);

        let mut beam = vec![Node { sim: init, path: Vec::new(), eaten: 0 }];
        let mut best: Option<(i64, Vec<u32>)> = None;
        let mut budget = PLAN_BUDGET;

        'depth: for _ in 0..PLAN_DEPTH {
            let mut next: Vec<(i64, Node)> = Vec::new();
            for node in beam.drain(..) {
                for (sc, d) in DIRS {
                    if budget <= 0 {
                        break 'depth;
                    }
                    budget -= 1;
                    let mut sim = node.sim.clone();
                    let out = sim.apply(d, &blocked);
                    if out == Outcome::Dead {
                        continue;
                    }
                    let eaten = node.eaten + i32::from(out == Outcome::Ate);
                    let mut path = node.path.clone();
                    path.push(sc);
                    let score = self.score_node(&sim, &blocked, &dist, eaten);
                    if best.as_ref().is_none_or(|(b, _)| score > *b) {
                        best = Some((score, path.clone()));
                    }
                    next.push((score, Node { sim, path, eaten }));
                }
            }
            if next.is_empty() {
                break;
            }
            next.sort_by(|a, b| b.0.cmp(&a.0));
            next.truncate(BEAM_WIDTH);
            beam = next.into_iter().map(|(_, n)| n).collect();
        }
        best.map(|(_, path)| path.into_iter().collect())
    }
}
