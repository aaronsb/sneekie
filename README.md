# Sneekie

A faithful **terminal** port of *Sneekie* — a snake game written in GW-BASIC in
July 1988 by Herbert Groot Jebbink (HerbySoft) and published on the MSX/MS-DOS
Computer Magazine diskette MCMPC-D2 (no. 25, October 1988).

The original ran in 80×25 text mode and POKE'd characters straight into video
memory. This port keeps that model exactly — a 4000-byte "VRAM" buffer rendered
through the **CP437** character set — so the box-drawing snake, the `♥`/`♣`
food, the `☺` traps, the `◙` pushable stones and the `↑→←` arrow hazards all
look just like 1988, in any modern terminal.

Ported from the [single-page HTML re-creation](https://herbert256.github.io/sneekie/)
of the original `SNEEKIE.BAS` — by Herbert Groot Jebbink, whose source lives at
[github.com/herbert256/sneekie](https://github.com/herbert256/sneekie). Code
comments carry the original BASIC line numbers so the lineage stays legible.

## Build & run

```sh
cargo run --release                            # boot menu: pick mode + movement
cargo run --release -- --classic --turn-based  # the 1988 feel
cargo run --release -- --plus --live           # survival, always gliding
cargo run --release -- --auto                  # watch the bot play
cargo run --release -- --auto --plus           # watch the bot survive the swarm
cargo run --release -- --theme amber
cargo run --release --features audio -- --plus # with real square-wave sound
```

Needs an **80×25** (or larger) terminal. If the window is too small it'll ask
you to enlarge it.

## Two axes: mode × movement

Launching with no flags shows a two-step CP437 boot menu. You pick a **mode**
and a **movement style**, independently — four combinations in all (or `A` for
autoplay, below).

**Mode** (`--classic` / `--plus`):
- **Classic** — no hunters; just clear the hearts.
- **Sneekie+** — survival mode with hunters (below).

**Movement** (`--turn-based` / `--live`):
- **Turn-based** — the snake steps once per keypress. In Sneekie+ the hunters
  move in lockstep with you, one step each time you step — pure tactical chess.
- **Live** — the snake is always gliding (the modern snake feel). In Sneekie+
  the hunters step on every glide-tick.

(With only one axis given as a flag, the other defaults sensibly: `--plus`
implies live, `--classic` implies turn-based.)

### Sneekie+ survival mode

Sneekie+ is a **wave system** layered over the level: a cycle between calm
heart-collecting and a "zombie" swarm.

- **Grace period.** Each wave opens calm — smileys are static `−50` traps and
  the field keeps seeding faces as you eat. **Bank as many points as you can**;
  the clock is shown on the spare bottom row. Grace is generous for the first
  wave (**25s**) and **tightens every cycle** (the swarm returns faster), down
  to an 8s floor.
- **The swarm wakes.** When the timer expires, the nearest smileys become
  **hunters** (`☻`, glowing red), the rest clear out, and **no new faces spawn**.
  Hunters step one cell toward your head on **every move** you make (in
  lockstep, if you're playing turn-based). Walls block them — use cover.
- **Banked points are your shield.** A hunter touching you — or you ramming one —
  **costs points and removes that hunter**, *if you can afford it* (the price is
  shown as "ram-kills" you can buy). Can't pay? It's death. So the danger phase
  is a race: keep eating faster than the hunters can reach you.
- **A score multiplier** climbs while the swarm is loose — ×2 the moment they
  wake, +1 each second — scaling the points you earn (not what you pay).
- **Clear the wave** (defeat every hunter) and you earn a breather: faces return,
  the multiplier resets, and a fresh — shorter — clock counts down to the next
  wave. Survive the cycles long enough to clear all the hearts and finish the
  level.

The field is thinned in Sneekie+ (vs. classic's dense scatter) so a level is
actually winnable under the pressure. Press **`m`** to toggle sound.

Balance lives in named constants at the top of `src/game/plus.rs`
(`GRACE_BASE`/`GRACE_STEP`/`GRACE_MIN`, `HUNTERS_MAX`, `HUNTER_COST`,
`WAVE_FACES`) and `glide_speed` in `src/game/play.rs` — easy to dial.

### Autoplay (the bot / screensaver)

`--auto` (or `A` in the menu, which then asks classic or Sneekie+) hands the
snake to a bot that plays — and, with its auto-restart, basically turns Sneekie
into a weird little screensaver (locked to glorious CGA color).

On the **static maze levels** the bot is driven by a bounded forward-search
**planner** (several cores of modern compute thrown at an 8086 GW-BASIC game).
Each replan it snapshots the board into a throwaway `Sim` and runs a budget-capped
**beam search** over move sequences, rolling each candidate forward and scoring
the result by — in priority order — food eaten, whether the head can still reach
its tail, distance to the nearest remaining heart, and open space. It commits to
the best plan and executes it tick-by-tick (running at full speed, not game-tick
speed), bailing out after a fixed number of simulated steps. Every committed move
still passes the **hard tail-safety gate** below, so the planner supplies the
routing while the proven one-ply check supplies the never-self-trap guarantee.

Where lookahead can't be trusted past one tick — **Sneekie+** (hunters move with
you) and the classic levels with **moving arrows** — the bot falls through to a
per-tick greedy chain instead. Each step:

1. **BFS** from the head to the nearest food (empty/heart/club cells), and take
   the first step of that shortest path —
2. **only if it's safe**: after the move, the head must still be able to reach
   its own tail (a tail-reachability BFS over a *virtual* body). Tail-chasing is
   always an escape, so this can never self-trap. A true Hamiltonian cycle would
   be provably immortal, but most Sneekie levels are mazes where no such cycle
   exists — tail-safety is the technique that works on arbitrary walls.
3. If the food step is unsafe, **chase the tail**: BFS to its own tail and step
   that way. Following the body out threads single-cell exits, so it extracts
   itself from a pocket instead of hugging the roomiest corner.
4. In Sneekie+, if it's cornered by hunters but has banked enough points, it
   **rams one** (paying the defeat cost) to punch a way out.
5. In **classic**, if it **stalls** (no score *gain* for ~13s — a maze it can't
   clear safely), it uses the F10 skip-level cheat so the show keeps moving.
   **Sneekie+ gets the same clock as a human** — full grace, no shortcut, no
   skipping. It plays the grace, then fights the swarm until it clears the level
   or is overwhelmed. If it ever wedges into a death-spiral the hunters can't
   reach, it does exactly what the legend tells a stuck human to do — presses
   **ESC** to give up a life and respawn (and autoplay restarts when lives run
   out).

On top of that, a short **move-history buffer** does cycle detection: it tracks
how long ago the head was last on each cell, and when a *stable* period recurs
more than once (a genuine loop, not a one-off backtrack) it steers toward the
least-recently-visited neighbor to break out early — before the slower stall
guards above have to fire.

Result: effectively immortal on the static classic levels, cycling through the
layouts indefinitely in CGA color; a fair, go-down-swinging run in Sneekie+.
Moving enemies (arrows, hunters) can still corner it — and that's fine, it just
restarts. Watch it play either mode (`--auto`, `--auto --plus`, or the menu's
`A`). Ctrl+C to quit. (`AUTO_STALL` in `src/game/autoplay/mod.rs` tunes patience;
`PLAN_BUDGET`/`PLAN_DEPTH`/`BEAM_WIDTH` in `src/game/autoplay/planner.rs` tune
how hard the planner thinks.)

## How to play

Eat every `♥` heart to clear a level (plus every `♣` club from level 17 on).
There are 32 levels across 8 maze layouts; the back 16 add moving hazards and
auto-advancing speed.

| Key | Action |
|-----|--------|
| Arrow keys | Steer the snake |
| `ESC` | Give up a life when you're stuck |
| `m` | Toggle sound (Sneekie+) |
| any key | Continue at a prompt |
| `1`–`4` | Switch theme live (Green / Amber / White / CGA) |
| `Ctrl+C` / `Ctrl+Q` | Quit |

Scoring: `♥` +10, `♣` +25, `☺` **−50** (avoid these!), `◙` pushable stone.
A per-level **Bonus** ticks down as you play and is added to your score when you
clear the level. Highscore, theme, and mute state persist to
`$XDG_CONFIG_HOME/sneekie/state` (`~/.config/sneekie/state`).

> There may also be a couple of function keys that do… helpful things. Shh.

## Themes

`hercules` (green), `amber`, `white`, `cga` (the colorized default). Pass with
`--theme <name>`, or press `1`–`4` at any prompt.

## Project layout

| File | Role |
|------|------|
| `src/main.rs` | Bootstrap: argument parsing, save path, help |
| `src/cp437.rs` | CP437 → Unicode table (the charset) |
| `src/theme.rs` | Color themes + CGA colorizer |
| `src/game/mod.rs` | The "machine": VRAM, CRT renderer, keyboard, BASIC output primitives |
| `src/game/layouts.rs` | The eight maze builders (`lay*`) |
| `src/game/enemies.rs` | The moving hazards (`sub*`) |
| `src/game/play.rs` | Level loop, movement, death, boot sequence |
| `src/game/plus.rs` | Sneekie+ survival mode and the boot menu |
| `src/game/autoplay/` | The self-driving bot — `mod.rs` (greedy chain + cycle/stall guards), `planner.rs` (bounded beam search), `sim.rs` (forward-model) |
| `src/game/audio.rs` | Square-wave synth (behind the `audio` feature) |

Classic Sneekie is fully preserved: every Sneekie+ hook in `play.rs` is guarded
by `self.plus`, and the score multiplier is a no-op (`mult == 1`) outside the
danger phase, so classic mode runs byte-identically to the pure port.

## Notes

- **Audio** is opt-in via the `audio` Cargo feature (rodio). The default build
  is dependency-free and silent; `--features audio` adds real square-wave tones
  through the existing `snd()` hooks, queued in sequence just like the original
  `SOUND` statements. Building with the feature needs ALSA dev headers on Linux
  (`pacman -S alsa-lib`); if no audio device is found at runtime the game falls
  back to silent.
- Original game © July 1988 by HerbySoft (Herbert Groot Jebbink) —
  [github.com/herbert256/sneekie](https://github.com/herbert256/sneekie). This
  port is an homage; all credit for the design is his.
