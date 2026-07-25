//! Live DPS overlay for Linux/Wine: connects directly to hook.dll over the
//! same TCP protocol the Tauri GUI uses, without any WebView2/Wine GUI
//! compositing involved. Launches `injector.exe` inside the game's Proton
//! prefix on startup, either directly via `wine` (preferred) or via
//! `protontricks-launch` (fallback, but has been observed to attach to a
//! separate, unrelated wineserver session rather than the game's live one on
//! at least Bazzite/CachyOS — see `spawn_injector_protontricks`'s doc comment).

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser as ClapParser;
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use engine::constants::CharacterType;
use engine::v1::Parser as EngineParser;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Terminal;
use tokio::net::TcpStream;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

/// Live GBFR Logs damage meter overlay for Linux/Wine.
///
/// Two ways to launch injector.exe inside the game's Wine prefix:
///
/// 1. `--wine`/`--wineprefix` (preferred): runs injector.exe directly via the
///    same wine binary and prefix the game itself uses, so it attaches to the
///    game's actual live wineserver session. Find these paths from the
///    running game process itself:
///
///        pgrep -f granblue_fantasy_relink.exe
///        cat /proc/<pid>/environ | tr '\0' '\n' | grep STEAM_COMPAT
///
///    --wineprefix is `${STEAM_COMPAT_DATA_PATH}/pfx`. --wine is the `wine`
///    binary next to the Proton build in STEAM_COMPAT_TOOL_PATHS (e.g.
///    ".../Proton-CachyOS Latest/files/bin/wine").
///
/// 2. `--appid` (fallback): uses `protontricks-launch`. Simpler, but observed
///    to attach to a separate, unrelated wineserver session rather than the
///    game's live one in at least one Bazzite/CachyOS setup — if injection
///    silently never finds the game process, switch to --wine/--wineprefix.
#[derive(clap::Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Path to injector.exe, run inside the game's Proton prefix.
    #[arg(long)]
    injector: PathBuf,

    /// Path to the wine binary from the game's own Proton build. Requires --wineprefix.
    #[arg(long)]
    wine: Option<PathBuf>,

    /// Path to the game's Wine prefix ("pfx") directory. Requires --wine.
    #[arg(long)]
    wineprefix: Option<PathBuf>,

    /// Steam AppID, used with protontricks-launch instead of --wine/--wineprefix.
    #[arg(long)]
    appid: Option<String>,
}

const LOG_FILE_NAME: &str = "tui.log";

/// Diagnostics (injector output, connection state) go to `tui.log` in the
/// working directory rather than an on-screen panel — `tail -f tui.log` in
/// another terminal if something needs debugging.
type SharedLog = Arc<Mutex<Option<File>>>;

fn open_log() -> SharedLog {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_FILE_NAME)
        .ok();

    Arc::new(Mutex::new(file))
}

fn push_log(log: &SharedLog, line: String) {
    if let Ok(mut file) = log.lock() {
        if let Some(file) = file.as_mut() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

fn read_lines(stream: impl Read, prefix: &'static str, log: SharedLog) {
    let reader = io::BufReader::new(stream);
    for line in reader.lines().map_while(Result::ok) {
        push_log(&log, format!("[{prefix}] {line}"));
    }
}

/// Spawns an already-configured `Command` (stdout/stderr must be piped),
/// forwarding its output into `log` so injection failures are visible in the
/// TUI without needing a separate log file. Returns the spawned process's PID
/// so it can be killed when `tui` exits — otherwise it leaks as an orphan
/// every time this program quits, which piles up fast across repeated runs.
fn spawn_and_track(mut command: std::process::Command, log: SharedLog) -> Option<u32> {
    let mut child = match command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(e) => {
            push_log(&log, format!("failed to launch injector: {e}"));
            return None;
        }
    };

    let pid = child.id();

    let stdout_handle = child
        .stdout
        .take()
        .map(|s| std::thread::spawn({
            let log = log.clone();
            move || read_lines(s, "out", log)
        }));
    let stderr_handle = child
        .stderr
        .take()
        .map(|s| std::thread::spawn({
            let log = log.clone();
            move || read_lines(s, "err", log)
        }));

    std::thread::spawn(move || {
        let status = child.wait();

        if let Some(handle) = stdout_handle {
            let _ = handle.join();
        }
        if let Some(handle) = stderr_handle {
            let _ = handle.join();
        }

        match status {
            Ok(status) => push_log(&log, format!("injector process exited: {status}")),
            Err(e) => push_log(&log, format!("failed to wait on injector process: {e}")),
        }
    });

    Some(pid)
}

/// Runs injector.exe directly via the game's own wine binary and prefix, so
/// it attaches to the game's actual live wineserver session instead of
/// whatever session protontricks-launch's sandboxing might spin up.
fn spawn_injector_direct(
    wine: PathBuf,
    wineprefix: PathBuf,
    injector: PathBuf,
    log: SharedLog,
) -> Option<u32> {
    let mut command = std::process::Command::new(wine);
    command.env("WINEPREFIX", wineprefix).arg(injector);
    spawn_and_track(command, log)
}

/// Runs injector.exe via `protontricks-launch --appid <appid> <injector>`.
///
/// This has been observed (on at least one Bazzite/CachyOS setup) to attach
/// to a separate, freshly-spawned wineserver rather than the game's actual
/// live session — confirmed by `wine tasklist` run the same way showing only
/// base OS processes, not the game, and by a second `wineserver` process
/// appearing the moment this runs. If injection silently never finds the
/// game process, switch to `spawn_injector_direct` instead.
fn spawn_injector_protontricks(appid: String, injector: PathBuf, log: SharedLog) -> Option<u32> {
    let mut command = std::process::Command::new("protontricks-launch");
    command.arg("--appid").arg(&appid).arg(&injector);
    spawn_and_track(command, log)
}

/// Connects to the hook's TCP socket and feeds decoded messages into the
/// shared parser, mirroring `connect_and_run_parser` in the Tauri app but
/// without any persistence or frontend push (the render loop just reads
/// `parser`'s state directly on a tick instead).
async fn run_network_loop(parser: Arc<Mutex<EngineParser>>, connected: Arc<AtomicBool>, log: SharedLog) {
    let mut logged_waiting = false;

    loop {
        match TcpStream::connect(protocol::SOCKET_ADDR).await {
            Ok(stream) => {
                connected.store(true, Ordering::Relaxed);
                logged_waiting = false;
                push_log(&log, format!("connected to hook at {}", protocol::SOCKET_ADDR));

                let decoder = LengthDelimitedCodec::new();
                let mut reader = FramedRead::new(stream, decoder);
                let mut inactivity_check = tokio::time::interval(Duration::from_secs(1));
                inactivity_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                loop {
                    tokio::select! {
                        message = reader.next() => {
                            let Some(Ok(msg)) = message else { break; };

                            if msg.is_empty() {
                                break;
                            }

                            if let Ok(msg) = protocol::bincode::deserialize::<protocol::Message>(&msg) {
                                let mut parser = parser.lock().unwrap();
                                match msg {
                                    protocol::Message::DamageEvent(event) => parser.on_damage_event(event),
                                    protocol::Message::OnAreaEnter(event) => parser.on_area_enter_event(event),
                                    protocol::Message::PlayerLoadEvent(event) => parser.on_player_load_event(event),
                                    protocol::Message::PlayerIdentityEvent(event) => parser.on_player_identity_event(event),
                                    protocol::Message::OnQuestComplete(event) => parser.on_quest_complete_event(event),
                                    protocol::Message::OnUpdateSBA(event) => parser.on_sba_update(event),
                                    protocol::Message::OnAttemptSBA(event) => parser.on_sba_attempt(event),
                                    protocol::Message::OnPerformSBA(event) => parser.on_sba_perform(event),
                                    protocol::Message::OnContinueSBAChain(event) => parser.on_continue_sba_chain(event),
                                    protocol::Message::OnDeathEvent(event) => parser.on_death_event(event),
                                    protocol::Message::OnBattleEnd => {
                                        parser.on_battle_end_event();
                                    }
                                }
                            }
                        }
                        _ = inactivity_check.tick() => {
                            let mut parser = parser.lock().unwrap();
                            parser.auto_save_if_inactive(chrono::Utc::now().timestamp_millis());
                        }
                    }
                }

                connected.store(false, Ordering::Relaxed);
                push_log(&log, "hook connection closed, waiting for game...".to_string());
            }
            Err(e) => {
                if !logged_waiting {
                    push_log(
                        &log,
                        format!("waiting for hook at {}: {e}", protocol::SOCKET_ADDR),
                    );
                    logged_waiting = true;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

fn run_ui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    parser: &Arc<Mutex<EngineParser>>,
    connected: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(1)])
                .split(frame.size());

            draw_meter(frame, chunks[0], parser);
            draw_status(frame, chunks[1], connected.load(Ordering::Relaxed));
        })?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => parser.lock().unwrap().manual_reset(),
                    _ => {}
                }
            }
        }
    }
}

fn draw_meter(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, parser: &Arc<Mutex<EngineParser>>) {
    let parser = parser.lock().unwrap();

    let mut players: Vec<_> = parser.derived_state.party.values().collect();
    players.sort_by(|a, b| b.total_damage.cmp(&a.total_damage));

    let rows = players.into_iter().map(|player| {
        Row::new(vec![
            Cell::from(player.character_type.friendly_name()),
            Cell::from(humanize_number(player.total_damage as f64)),
            Cell::from(humanize_number(player.dps)),
        ])
        .style(Style::default().fg(character_color(&player.character_type)))
    });

    let header = Row::new(vec!["Character", "Damage", "DPS (dmg/s)"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("GBFR Logs — Meter"));

    frame.render_widget(table, area);
}

/// Formats large numbers with a K/M/B unit suffix so damage and DPS columns
/// stay readable instead of running into the millions.
fn humanize_number(value: f64) -> String {
    const UNITS: [(f64, &str); 3] = [(1_000_000_000.0, "B"), (1_000_000.0, "M"), (1_000.0, "K")];

    for (threshold, suffix) in UNITS {
        if value.abs() >= threshold {
            return format!("{:.2}{suffix}", value / threshold);
        }
    }

    format!("{value:.0}")
}

/// Assigns each character a distinct, stable color by spacing them evenly
/// around the hue wheel in their declaration order — simpler than curating
/// 30+ colors by hand, and still keeps any given party's rows distinguishable.
fn character_color(character_type: &CharacterType) -> Color {
    use CharacterType::*;

    let index = match character_type {
        Pl0000 => 0,
        Pl0100 => 1,
        Pl0200 => 2,
        Pl0300 => 3,
        Pl0400 => 4,
        Pl0500 => 5,
        Pl0600 => 6,
        Pl0700 => 7,
        Pl0800 => 8,
        Pl0900 => 9,
        Pl1000 => 10,
        Pl1100 => 11,
        Pl1200 => 12,
        Pl1300 => 13,
        Pl1400 => 14,
        Pl1500 => 15,
        Pl1600 => 16,
        Pl1700 => 17,
        Pl1800 => 18,
        Pl1900 => 19,
        Pl2000 => 20,
        Pl2100 => 21,
        Pl2200 => 22,
        Pl2300 => 23,
        Pl2400 => 24,
        Pl2500 => 25,
        Pl2600 => 26,
        Pl2700 => 27,
        Pl2800 => 28,
        Pl2900 => 29,
        Pl0700Ghost => 30,
        Pl0700GhostSatellite => 31,
        Unknown(_) => return Color::Gray,
    };

    hue_wheel_color(index, 32)
}

/// Spreads declaration-order-adjacent characters apart on the hue wheel
/// instead of placing them next to each other, where they'd be nearly
/// indistinguishable (e.g. Eustace and Fraux at indices 27/28 of 32).
/// 7 is coprime with 32, so `index * 7 mod 32` visits every slot while
/// keeping consecutive indices far apart in hue.
fn hue_wheel_color(index: usize, total: usize) -> Color {
    let step = 7;
    let spread_index = (index * step) % total;
    let hue = (spread_index as f32 / total as f32) * 360.0;
    let (r, g, b) = hsv_to_rgb(hue, 0.65, 0.95);
    Color::Rgb(r, g, b)
}

fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> (u8, u8, u8) {
    let c = value * saturation;
    let h_prime = hue / 60.0;
    let x = c * (1.0 - (h_prime.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = value - c;
    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

fn draw_status(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, connected: bool) {
    let status = if connected {
        "Connected to game — r: reset meter, q: quit"
    } else {
        "Waiting for game... — r: reset meter, q: quit"
    };

    frame.render_widget(Paragraph::new(status), area);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let log = open_log();
    push_log(&log, format!("starting tui, logging to {LOG_FILE_NAME}"));

    let injector_pid = match (args.wine, args.wineprefix, args.appid) {
        (Some(wine), Some(wineprefix), _) => {
            push_log(&log, format!("launching injector directly via {wine:?} (prefix {wineprefix:?})"));
            spawn_injector_direct(wine, wineprefix, args.injector, log.clone())
        }
        (_, _, Some(appid)) => {
            push_log(&log, format!("launching injector via protontricks-launch --appid {appid}"));
            spawn_injector_protontricks(appid, args.injector, log.clone())
        }
        _ => {
            push_log(
                &log,
                "no launch method given: pass either --wine + --wineprefix, or --appid".to_string(),
            );
            None
        }
    };

    let parser = Arc::new(Mutex::new(EngineParser::new(None, None)));
    let connected = Arc::new(AtomicBool::new(false));

    tokio::spawn(run_network_loop(parser.clone(), connected.clone(), log.clone()));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_ui(&mut terminal, &parser, &connected);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Best-effort: kill the protontricks-launch process we spawned so it
    // doesn't leak as an orphan. See spawn_injector's doc comment for why
    // this can't reach the deeper injector.exe process too.
    if let Some(pid) = injector_pid {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
    }

    result
}
