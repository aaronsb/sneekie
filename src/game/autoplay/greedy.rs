//! The per-tick greedy fallback chain.
//!
//! When neither planner is in charge — or when the planner finds nothing safe —
//! the bot falls back to this cheap, memoryless navigator. In priority order:
//! chase the nearest food (if the step stays tail-safe), ram an adjacent hunter
//! (Sneekie+), chase the tail to thread out of a pocket, else head for the most
//! open space. A short move-history ring (`trail_ago`/`perturb_move`) nudges it
//! off repeating cycles. These are the techniques that kept the bot alive before
//! the [`super::planner`] and [`super::mcts`] brains were layered on top.

use std::collections::VecDeque;

use super::{reverse, DIRS, TRAIL};

impl crate::game::Game {
    /// Steps since the head was last on `cell` within the ring (0 = not found).
    pub(super) fn trail_ago(&self, cell: i32) -> usize {
        for j in 1..=TRAIL {
            let idx = (self.auto_trail_i + TRAIL - j) % TRAIL;
            if self.auto_trail[idx] == cell {
                return j;
            }
        }
        0
    }

    /// Pick the passable, non-reverse neighbor the head has visited least in its
    /// recent trail (ties broken by open space) — a nudge toward fresh ground to
    /// break a repeating pattern.
    pub(super) fn perturb_move(&self, head: i32) -> Option<u32> {
        let rev = reverse(self.e);
        let mut best: Option<u32> = None;
        let mut best_key = (usize::MAX, i32::MIN);
        for (sc, d) in DIRS {
            if sc == rev {
                continue;
            }
            let n = head + d;
            if n < 0 || (n as usize) >= 4000 || !self.passable(n) {
                continue;
            }
            let visits = self.auto_trail.iter().filter(|&&c| c == n).count();
            let key = (visits, -self.flood_count(n)); // fewest visits, then most room
            if best.is_none() || key < best_key {
                best_key = key;
                best = Some(sc);
            }
        }
        best
    }

    /// Choose the next direction (a scan code: 72/80/75/77).
    ///
    /// The hybrid: chase the nearest food greedily, but only commit to a step
    /// that leaves the head still able to reach its own tail (tail-reachability
    /// — the snake can then always escape by following its tail). If the food
    /// step is unsafe, take the safe move that keeps the most room. This makes
    /// it effectively immortal on the static levels; moving enemies (arrows,
    /// hunters) can still corner it, and autoplay just restarts.
    pub(super) fn auto_dir(&self) -> u32 {
        let head = self.t[self.btel as usize];
        let rev = reverse(self.e);

        // 1. Greedy toward food — if that first step keeps the tail reachable.
        if let Some(sc) = self.bfs_food(head, rev) {
            if self.is_safe_move(sc) {
                return sc;
            }
        }
        // 2. Sneekie+: a hunter is right next to us and we can pay? ram it.
        //    Fighting the swarm proactively keeps tight grooves from forming.
        if self.plus && self.can_defeat() {
            for (sc, d) in DIRS {
                if sc == rev {
                    continue;
                }
                let n = head + d;
                if n >= 0 && (n as usize) < 4000 && self.peek(n) == 2 {
                    return sc;
                }
            }
        }
        // 3. Chase the tail: BFS to the tail through open cells and step that
        //    way. Following the body out threads single-cell exits, so the
        //    snake extracts itself from a pocket instead of hugging the
        //    roomiest corner.
        if let Some(sc) = self.bfs_to_tail(head, rev) {
            return sc;
        }
        // 4. Last resort: the most open cell and hope.
        self.most_open(head, rev)
    }

    /// BFS from the head through open cells to the snake's own tail; returns the
    /// first step of that path. The returned step is always into a passable
    /// cell (the tail is only ever *reached from* one).
    fn bfs_to_tail(&self, head: i32, rev: u32) -> Option<u32> {
        let tail = self.t[self.etel as usize];
        let mut seen = vec![false; 4000];
        let mut first = vec![0u32; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        seen[head as usize] = true;
        for (sc, d) in DIRS {
            if sc == rev {
                continue;
            }
            let n = head + d;
            if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && self.passable(n) {
                seen[n as usize] = true;
                first[n as usize] = sc;
                q.push_back(n);
            }
        }
        while let Some(c) = q.pop_front() {
            for (_sc, d) in DIRS {
                let n = c + d;
                if n < 0 || (n as usize) >= 4000 || seen[n as usize] {
                    continue;
                }
                if n == tail {
                    return Some(first[c as usize]);
                }
                if self.passable(n) {
                    seen[n as usize] = true;
                    first[n as usize] = first[c as usize];
                    q.push_back(n);
                }
            }
        }
        None
    }

    /// BFS from the head to the nearest food; returns the first step's scan code.
    fn bfs_food(&self, head: i32, rev: u32) -> Option<u32> {
        let mut seen = vec![false; 4000];
        let mut first = vec![0u32; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        seen[head as usize] = true;
        for (sc, d) in DIRS {
            if sc == rev {
                continue; // can't reverse into the neck
            }
            let n = head + d;
            if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && self.passable(n) {
                seen[n as usize] = true;
                first[n as usize] = sc;
                q.push_back(n);
            }
        }
        while let Some(c) = q.pop_front() {
            if self.is_food(c) {
                return Some(first[c as usize]);
            }
            for (_sc, d) in DIRS {
                let n = c + d;
                if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && self.passable(n) {
                    seen[n as usize] = true;
                    first[n as usize] = first[c as usize];
                    q.push_back(n);
                }
            }
        }
        None
    }

    /// Pick the non-reverse neighbor opening onto the most reachable space.
    fn most_open(&self, head: i32, rev: u32) -> u32 {
        let mut best = self.e;
        let mut best_space = -1;
        for (sc, d) in DIRS {
            if sc == rev {
                continue;
            }
            let n = head + d;
            if n >= 0 && (n as usize) < 4000 && self.passable(n) {
                let space = self.flood_count(n);
                if space > best_space {
                    best_space = space;
                    best = sc;
                }
            }
        }
        best
    }

    /// Count reachable passable cells from `start` (capped, for the fallback).
    fn flood_count(&self, start: i32) -> i32 {
        let mut seen = vec![false; 4000];
        let mut q: VecDeque<i32> = VecDeque::new();
        seen[start as usize] = true;
        q.push_back(start);
        let mut count = 0;
        while let Some(c) = q.pop_front() {
            count += 1;
            if count > 400 {
                break;
            }
            for (_sc, d) in DIRS {
                let n = c + d;
                if n >= 0 && (n as usize) < 4000 && !seen[n as usize] && self.passable(n) {
                    seen[n as usize] = true;
                    q.push_back(n);
                }
            }
        }
        count
    }
}
