//! An AlphaGo-shaped, multi-core MCTS planner for the Sneekie+ danger phase.
//!
//! When the swarm is loose the board is the one place lookahead truly pays:
//! hunters are a quasi-adversary, and (because no new faces spawn while they
//! hunt) their motion is *deterministic* given the snake's moves — so it can be
//! searched. This is Monte-Carlo Tree Search in the AlphaGo mould, minus the
//! learned nets: **PUCT** selection, a hand-rolled **policy prior** (a softmax
//! over food-proximity and hunter-distance) standing in for the policy network,
//! and **hunter-aware ε-greedy rollouts** scored by survival-and-eating standing
//! in for the value network.
//!
//! **Root parallelization.** Each of N worker cores grows its *own* tree from
//! the same root and the visit counts are merged at the end (the move the most
//! workers favored wins). The rollouts are stochastic — each worker carries its
//! own seeded PRNG — so the trees genuinely diverge and N cores explore N times
//! the surface area. Several cores of modern compute, finally, brought to bear on
//! a 1988 GW-BASIC snake; `--planner-cores` (or in-game `+`/`-`) sets N.

use std::collections::{HashMap, HashSet, VecDeque};

use rayon::prelude::*;

use super::sim::{Outcome, Sim};
use super::DIRS;

/// Tree simulations per worker per move. Total search ≈ this × cores, so adding
/// cores buys more surface area rather than just finishing sooner.
pub(super) const MCTS_SIMS: u32 = 1200;
/// How many moves each rollout plays before scoring the resulting state.
const ROLLOUT_DEPTH: usize = 40;
/// PUCT exploration constant: higher widens the search, lower sharpens it.
const C_PUCT: f64 = 1.3;
/// Rollout exploration: this fraction of rollout steps take a random legal move
/// instead of the greedy one, so workers' trees diverge and parallelism pays.
const ROLLOUT_EPS_NUM: u64 = 1;
const ROLLOUT_EPS_DEN: u64 = 4;

/// The read-only board the workers share: the static-obstacle map (hunters left
/// passable — they live in the `Sim`) and the food-distance field. `Send + Sync`
/// by construction, so it can be borrowed across rayon worker threads.
pub(super) struct PlanCtx {
    blocked: Vec<bool>,
    dist: Vec<i32>,
}

/// A tiny xorshift PRNG, one per worker, seeded distinctly so the rollouts (and
/// thus the trees) diverge across cores.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn roll_eps(&mut self) -> bool {
        self.next() % ROLLOUT_EPS_DEN < ROLLOUT_EPS_NUM
    }
}

struct Node {
    sim: Sim,
    parent: usize,                 // index in the arena (root points to itself)
    mv: u32,                       // scan code of the move from the parent
    prior: f64,                    // policy prior P(a) for that move
    children: Vec<usize>,          // expanded child indices
    untried: Vec<(u32, i32, f64)>, // (scan code, offset delta, prior) not yet expanded
    n: u32,                        // visit count
    w: f64,                        // accumulated value
    terminal: Option<f64>,         // Some(value) once dead (0.0) or cleared (1.0)
}

/// Map a rolled-out state to [0, 1]. The winning strategy under the swarm is to
/// **out-eat it**: every heart banks points, the bank funds ram-kills, and
/// clearing the hearts ends the level — so *eating* is the dominant productive
/// term. Survival is the gate beneath it (a line that dies early keeps only a
/// fraction of the survival credit), and proximity to the nearest heart breaks
/// ties so the search closes on food rather than twirling in a safe pocket.
fn value(steps: usize, eaten: i32, start_food: usize, end_dist: i32) -> f64 {
    let surv = steps as f64 / ROLLOUT_DEPTH as f64;
    let eat = if start_food > 0 {
        (eaten as f64 / start_food as f64).min(1.0)
    } else {
        0.0
    };
    let prox = if end_dist == i32::MAX {
        0.0
    } else {
        1.0 - (end_dist.min(60) as f64 / 60.0)
    };
    (0.30 * surv + 0.50 * eat + 0.20 * prox).clamp(0.0, 1.0)
}

/// Build a fresh node: the legal (non-reverse) moves and a softmax policy prior
/// over them from food-proximity and hunter-distance.
fn node(ctx: &PlanCtx, sim: Sim, parent: usize, mv: u32, prior: f64) -> Node {
    let rev = -sim.last_dir();
    let head = sim.head();
    let mut raw: Vec<(u32, i32, f64)> = Vec::new();
    for (sc, d) in DIRS {
        if d == rev {
            continue;
        }
        let n = head + d;
        let fd = if n >= 0 && (n as usize) < 4000 {
            let v = ctx.dist[n as usize];
            if v == i32::MAX {
                300
            } else {
                v
            }
        } else {
            300
        };
        let hd = sim.nearest_hunter_dist(n).min(20);
        raw.push((sc, d, -(fd as f64) + 2.0 * hd as f64));
    }
    let maxr = raw.iter().map(|x| x.2).fold(f64::MIN, f64::max);
    let sum: f64 = raw.iter().map(|x| (x.2 - maxr).exp()).sum();
    let untried: Vec<(u32, i32, f64)> = raw
        .iter()
        .map(|&(sc, d, r)| (sc, d, (r - maxr).exp() / sum))
        .collect();
    Node {
        sim,
        parent,
        mv,
        prior,
        children: Vec::new(),
        untried,
        n: 0,
        w: 0.0,
        terminal: None,
    }
}

/// PUCT: pick the child maximizing Q + c·P·√N_parent / (1 + N_child).
fn select(arena: &[Node], ni: usize) -> usize {
    let sqrt_parent = (arena[ni].n.max(1) as f64).sqrt();
    let mut best = (f64::MIN, arena[ni].children[0]);
    for &ci in &arena[ni].children {
        let c = &arena[ci];
        let q = if c.n > 0 { c.w / c.n as f64 } else { 0.0 };
        let u = C_PUCT * c.prior * sqrt_parent / (1.0 + c.n as f64);
        if q + u > best.0 {
            best = (q + u, ci);
        }
    }
    best.1
}

/// A heavy ε-greedy rollout: mostly flee hunters / approach food, but take a
/// random legal move a fraction of the time so workers diverge. Plays until
/// death, clear, or depth, then scores the result.
fn rollout(ctx: &PlanCtx, mut sim: Sim, rng: &mut Rng) -> f64 {
    let start_food = sim.food_left();
    let mut eaten = 0;
    for step in 0..ROLLOUT_DEPTH {
        if sim.food_left() == 0 {
            return 1.0; // cleared the level
        }
        let rev = -sim.last_dir();
        let head = sim.head();
        let mut legal: Vec<(i32, f64)> = Vec::new(); // (delta, heuristic)
        for (_sc, d) in DIRS {
            if d == rev {
                continue;
            }
            let n = head + d;
            if !sim.enterable(n, &ctx.blocked) {
                continue;
            }
            let fd = if (n as usize) < 4000 && ctx.dist[n as usize] != i32::MAX {
                ctx.dist[n as usize]
            } else {
                300
            };
            let hd = sim.nearest_hunter_dist(n).min(20);
            legal.push((d, -(fd as f64) + 2.0 * hd as f64));
        }
        if legal.is_empty() {
            return value(step, eaten, start_food, ctx.dist[head as usize]); // boxed in
        }
        let d = if rng.roll_eps() {
            legal[rng.below(legal.len())].0
        } else {
            legal
                .iter()
                .copied()
                .fold((i32::MIN, f64::MIN), |best, (d, h)| {
                    if h > best.1 {
                        (d, h)
                    } else {
                        best
                    }
                })
                .0
        };
        match sim.apply_danger(d, &ctx.blocked) {
            Outcome::Dead => return value(step, eaten, start_food, ctx.dist[head as usize]),
            Outcome::Ate => eaten += 1,
            // No smileys exist once the swarm is loose, so apply_danger never
            // yields Penalty; handle it for exhaustiveness.
            Outcome::Moved | Outcome::Penalty => {}
        }
    }
    value(ROLLOUT_DEPTH, eaten, start_food, ctx.dist[sim.head() as usize])
}

/// Grow one independent search tree of `sims` simulations and return its
/// root-child visit tallies `(move, visits)` plus the node count searched.
fn search_tree(ctx: &PlanCtx, root: &Sim, sims: u32, seed: u64) -> (Vec<(u32, u32)>, usize) {
    let mut rng = Rng::new(seed);
    let mut arena: Vec<Node> = Vec::with_capacity(sims as usize + 8);
    arena.push(node(ctx, root.clone(), 0, 0, 1.0));

    for _ in 0..sims {
        // selection
        let mut ni = 0usize;
        while arena[ni].terminal.is_none()
            && arena[ni].untried.is_empty()
            && !arena[ni].children.is_empty()
        {
            ni = select(&arena, ni);
        }
        // expansion + simulation
        let leaf_value = if let Some(v) = arena[ni].terminal {
            v
        } else if let Some((sc, d, prior)) = arena[ni].untried.pop() {
            let mut sim = arena[ni].sim.clone();
            let out = sim.apply_danger(d, &ctx.blocked);
            let mut child = node(ctx, sim, ni, sc, prior);
            if out == Outcome::Dead {
                child.terminal = Some(0.0);
            } else if child.sim.food_left() == 0 {
                child.terminal = Some(1.0);
            }
            let cidx = arena.len();
            let v = child
                .terminal
                .unwrap_or_else(|| rollout(ctx, child.sim.clone(), &mut rng));
            arena.push(child);
            arena[ni].children.push(cidx);
            ni = cidx;
            v
        } else {
            arena[ni].terminal.unwrap_or(0.0)
        };
        // backpropagation
        let mut cur = ni;
        loop {
            arena[cur].n += 1;
            arena[cur].w += leaf_value;
            if cur == 0 {
                break;
            }
            cur = arena[cur].parent;
        }
    }

    let tally = arena[0]
        .children
        .iter()
        .map(|&ci| (arena[ci].mv, arena[ci].n))
        .collect();
    (tally, arena.len())
}

impl crate::game::Game {
    /// Static-obstacle map for the danger phase. Identical to the planner's, but
    /// hunters (`2`) are left *passable* — they're tracked dynamically in the
    /// `Sim` so the search can move them, not frozen as walls.
    fn build_blocked_dynamic(&self) -> Vec<bool> {
        let mut b = vec![false; 4000];
        for off in (0..4000i32).step_by(2) {
            b[off as usize] = self.static_blocked(off) && self.peek(off) != 2;
        }
        b
    }

    /// Snapshot the live danger-phase board into a `Sim`: body, food, the current
    /// hunters, and the score available to spend on ram/defeat costs.
    fn build_danger_sim(&self) -> Sim {
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
        let mut sim = Sim::new(body, food);
        sim.set_hunters(self.hunters.clone(), self.zcore);
        sim
    }

    /// Run root-parallel MCTS across `planner_cores` workers and return the move
    /// the merged search favors most, or `None` if there's nothing to search.
    /// Records the total nodes searched for the HUD.
    pub(super) fn plan_mcts(&mut self) -> Option<u32> {
        let blocked = self.build_blocked_dynamic();
        let dist = self.food_dist_field(&blocked);
        let ctx = PlanCtx { blocked, dist };
        let root = self.build_danger_sim();
        if root.food_left() == 0 {
            return None;
        }

        let cores = self.planner_cores.max(1);
        let base = self.rng | 1;
        // Each worker grows an independent tree with a distinct seed; rollouts
        // are stochastic, so the trees diverge and the cores add real coverage.
        let results: Vec<(Vec<(u32, u32)>, usize)> = (0..cores)
            .into_par_iter()
            .map(|k| {
                let seed = base
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    .wrapping_add(k as u64 + 1);
                search_tree(&ctx, &root, MCTS_SIMS, seed)
            })
            .collect();

        // Merge: the move the workers collectively visited most wins.
        let mut votes: HashMap<u32, u32> = HashMap::new();
        let mut nodes = 0usize;
        for (tally, n) in &results {
            nodes += n;
            for &(mv, v) in tally {
                *votes.entry(mv).or_insert(0) += v;
            }
        }
        self.plan_nodes = nodes;
        votes.into_iter().max_by_key(|&(_, v)| v).map(|(mv, _)| mv)
    }
}
