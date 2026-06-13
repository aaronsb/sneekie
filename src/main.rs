//! SNEEKIE — (c) July '88 by HerbySoft (Herbert Groot Jebbink)
//!
//! A faithful terminal port of the 1988 GW-BASIC snake game, by way of the
//! single-page HTML re-creation. This file is just the bootstrap: argument
//! parsing, save-file location, and the help text. The game lives in
//! [`game::Game`]; the charset and palettes are in [`cp437`] and [`theme`].

mod cp437;
mod game;
mod theme;

use std::io;

use game::Game;
use theme::theme_index;

/// Where to persist the highscore and chosen theme: `$XDG_CONFIG_HOME/sneekie/state`
/// (falling back to `~/.config/sneekie/state`).
fn save_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("sneekie").join("state"))
}

fn print_help() {
    println!(
        "Sneekie — a 1988 GW-BASIC snake game, reborn for the terminal.\n\
         \n\
         USAGE:\n\
         \x20   sneekie [--classic|--plus] [--turn-based|--live] [--auto] [--theme <name>]\n\
         \n\
         MODE (no flag shows a boot menu):\n\
         \x20   --classic   the 1988 game, no hunters\n\
         \x20   --plus      Sneekie+ survival: smileys become \u{263B} hunters that\n\
         \x20               chase you after a grace timer; a score x multiplier\n\
         \x20               climbs the longer you brave the swarm\n\
         \x20   --auto      a bot plays (BFS-to-food + open-space fallback);\n\
         \x20               combine with --plus to watch it survive the swarm\n\
         \n\
         MOVEMENT:\n\
         \x20   --turn-based   the snake steps once per keypress (hunters move\n\
         \x20                  in lockstep with you)\n\
         \x20   --live         the snake is always gliding (hunters step each tick)\n\
         \n\
         THEMES:\n\
         \x20   hercules  amber  white  cga   (default: cga)\n\
         \x20   or press 1-4 at any prompt to switch live.\n\
         \n\
         CONTROLS:\n\
         \x20   Arrow keys        steer the snake\n\
         \x20   ESC               give up a life when stuck\n\
         \x20   m                 toggle sound (Sneekie+)\n\
         \x20   any key           continue at a prompt\n\
         \x20   Ctrl+C / Ctrl+Q   quit\n\
         \n\
         GOAL: eat every \u{2665} heart (and \u{2663} club on levels 17+) to clear a level.\n\
         \u{2665} = +10   \u{2663} = +25   \u{263A} = -50 (avoid!)   \u{25D9} = pushable stone\n\
         \n\
         AUDIO: build with `--features audio` for real square-wave sound.\n\
         Needs an 80x25 terminal. Original (c) July '88 by HerbySoft."
    );
}

fn main() -> io::Result<()> {
    // ---- argument parsing ----
    let mut forced_theme: Option<usize> = None;
    let mut forced_mode: Option<bool> = None; // Some(true)=+, Some(false)=classic
    let mut forced_live: Option<bool> = None; // Some(true)=live, Some(false)=turn-based
    let mut forced_auto = false; // --auto: the bot drives
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-v" | "--version" => {
                println!("sneekie {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--plus" | "+" => {
                forced_mode = Some(true);
            }
            "--classic" => {
                forced_mode = Some(false);
            }
            "--live" | "--snake" => {
                forced_live = Some(true);
            }
            "--turn-based" | "--turn" => {
                forced_live = Some(false);
            }
            "--auto" | "--autoplay" => {
                forced_auto = true;
            }
            "--theme" => {
                if let Some(name) = args.get(i + 1) {
                    forced_theme = Some(theme_index(name));
                    i += 1;
                }
            }
            s if s.starts_with("--theme=") => {
                forced_theme = Some(theme_index(&s["--theme=".len()..]));
            }
            "hercules" | "amber" | "white" | "cga" => {
                forced_theme = Some(theme_index(arg));
            }
            other => {
                eprintln!("sneekie: unknown argument '{}'. Try --help.", other);
                return Ok(());
            }
        }
        i += 1;
    }

    let mut game = Game::new(forced_theme, forced_mode, forced_live, forced_auto, save_path());
    game.init_terminal()?;
    game.ensure_size();
    game.program();

    Ok(())
}
