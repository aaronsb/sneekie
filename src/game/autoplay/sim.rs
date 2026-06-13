//! A throwaway forward-simulation of the board, for autoplay lookahead.
//!
//! [`Sim`] is a cheap, mutable copy of just what the planner needs to roll the
//! game forward without touching live VRAM: the snake body (a deque, tail at the
//! front, head at the back), an occupancy set for O(1) self-collision tests, and
//! the set of remaining food cells. Static obstacles (walls, stones, smileys,
//! arrows and — in Sneekie+ — hunters) live in an immutable `blocked` map the
//! planner builds once per replan and threads through by reference, so cloning a
//! `Sim` only copies the small mutable parts.

use std::collections::{HashSet, VecDeque};

use super::DIRS;

/// What one simulated step did.
#[derive(PartialEq, Clone, Copy)]
pub(super) enum Outcome {
    Moved,
    Ate,
    Dead,
}

#[derive(Clone)]
pub(super) struct Sim {
    body: VecDeque<i32>, // front = tail, back = head
    occ: HashSet<i32>,   // body occupancy (fast collision tests)
    food: HashSet<i32>,  // remaining heart/club cells
}

impl Sim {
    pub(super) fn new(body: VecDeque<i32>, food: HashSet<i32>) -> Self {
        let occ: HashSet<i32> = body.iter().copied().collect();
        Sim { body, occ, food }
    }

    pub(super) fn head(&self) -> i32 {
        *self.body.back().unwrap()
    }

    pub(super) fn food_left(&self) -> usize {
        self.food.len()
    }

    /// Roll one step by VRAM-offset delta `d`. `blocked` is the static-obstacle
    /// map (true = impassable). Mutates the sim and reports what happened.
    pub(super) fn apply(&mut self, d: i32, blocked: &[bool]) -> Outcome {
        let next = self.head() + d;
        if next < 0 || (next as usize) >= blocked.len() || blocked[next as usize] {
            return Outcome::Dead;
        }
        let eating = self.food.contains(&next);
        // Unless we grow, the tail vacates first — so stepping into the cell the
        // tail is leaving is legal (standard snake rule).
        if !eating {
            if let Some(tail) = self.body.pop_front() {
                self.occ.remove(&tail);
            }
        }
        if self.occ.contains(&next) {
            return Outcome::Dead; // ran into the body (incl. reversing into the neck)
        }
        self.occ.insert(next);
        self.body.push_back(next);
        if eating {
            self.food.remove(&next);
            Outcome::Ate
        } else {
            Outcome::Moved
        }
    }

    /// A cell the snake could move through right now: in bounds, not a static
    /// obstacle, not currently part of the body.
    fn free(&self, off: i32, blocked: &[bool]) -> bool {
        off >= 0
            && (off as usize) < blocked.len()
            && !blocked[off as usize]
            && !self.occ.contains(&off)
    }

    /// Can the head still reach its own tail over free cells? If so the snake can
    /// always escape by following its tail, so it can't have boxed itself in.
    /// Early-exits on success and is capped, so a hopeless search stays cheap
    /// (returning `false` — conservatively "unsafe").
    pub(super) fn tail_reachable(&self, blocked: &[bool]) -> bool {
        let head = self.head();
        let tail = *self.body.front().unwrap();
        if head == tail {
            return true;
        }
        let mut seen: HashSet<i32> = HashSet::new();
        let mut q: VecDeque<i32> = VecDeque::new();
        seen.insert(head);
        q.push_back(head);
        let mut visited = 0;
        while let Some(c) = q.pop_front() {
            visited += 1;
            if visited > 900 {
                return false;
            }
            for (_sc, d) in DIRS {
                let n = c + d;
                if n == tail {
                    return true;
                }
                if self.free(n, blocked) && seen.insert(n) {
                    q.push_back(n);
                }
            }
        }
        false
    }

    /// Reachable free space from the head (capped) — a roominess tiebreak that
    /// keeps the bot out of dead-ending corridors.
    pub(super) fn open_space(&self, blocked: &[bool]) -> i32 {
        let head = self.head();
        let mut seen: HashSet<i32> = HashSet::new();
        let mut q: VecDeque<i32> = VecDeque::new();
        seen.insert(head);
        q.push_back(head);
        let mut count = 0;
        while let Some(c) = q.pop_front() {
            count += 1;
            if count > 200 {
                break;
            }
            for (_sc, d) in DIRS {
                let n = c + d;
                if self.free(n, blocked) && seen.insert(n) {
                    q.push_back(n);
                }
            }
        }
        count
    }
}
