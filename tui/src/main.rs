//! Live DPS overlay for Linux/Wine: connects directly to hook.dll over the
//! same TCP protocol the Tauri GUI uses, without any WebView2/Wine GUI
//! compositing involved. Launches `injector.exe` inside the game's Proton
//! prefix via `protontricks-launch` on startup.

use std::collections::VecDeque;
use std::io::{self, BufRead, Read};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser as ClapParser;
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use engine::v1::Parser as EngineParser;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Terminal;
use tokio::net::TcpStream;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

/// Live GBFR Logs damage meter overlay for Linux/Wine.
#[derive(clap::Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Steam AppID of Granblue Fantasy Relink, used to find its Proton prefix.
    #[arg(long)]
    appid: String,

    /// Path to injector.exe, run inside the game's Proton prefix via protontricks-launch.
    #[arg(long)]
    injector: PathBuf,
}

const MAX_LOG_LINES: usize = 200;

type SharedLog = Arc<Mutex<VecDeque<String>>>;

fn push_log(log: &SharedLog, line: String) {
    let mut log = log.lock().unwrap();
    if log.len() >= MAX_LOG_LINES {
        log.pop_front();
    }
    log.push_back(line);
}

fn read_lines(stream: impl Read, prefix: &'static str, log: SharedLog) {
    let reader = io::BufReader::new(stream);
    for line in reader.lines().map_while(Result::ok) {
        push_log(&log, format!("[{prefix}] {line}"));
    }
}

/// Spawns `protontricks-launch --appid <appid> <injector>` in the background,
/// forwarding its stdout/stderr into `log` so injection failures are visible
/// in the TUI without needing a separate log file.
fn spawn_injector(appid: String, injector: PathBuf, log: SharedLog) {
    std::thread::spawn(move || {
        let mut child = match std::process::Command::new("protontricks-launch")
            .arg("--appid")
            .arg(&appid)
            .arg(&injector)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                push_log(&log, format!("failed to launch protontricks-launch: {e}"));
                return;
            }
        };

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
}

/// Connects to the hook's TCP socket and feeds decoded messages into the
/// shared parser, mirroring `connect_and_run_parser` in the Tauri app but
/// without any persistence or frontend push (the render loop just reads
/// `parser`'s state directly on a tick instead).
async fn run_network_loop(parser: Arc<Mutex<EngineParser>>, connected: Arc<AtomicBool>) {
    loop {
        match TcpStream::connect(protocol::SOCKET_ADDR).await {
            Ok(stream) => {
                connected.store(true, Ordering::Relaxed);

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
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

fn run_ui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    parser: &Arc<Mutex<EngineParser>>,
    connected: &Arc<AtomicBool>,
    log: &SharedLog,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(8), Constraint::Length(1)])
                .split(frame.size());

            draw_meter(frame, chunks[0], parser);
            draw_log(frame, chunks[1], log);
            draw_status(frame, chunks[2], connected.load(Ordering::Relaxed));
        })?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    return Ok(());
                }
            }
        }
    }
}

fn draw_meter(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, parser: &Arc<Mutex<EngineParser>>) {
    let parser = parser.lock().unwrap();

    let names: std::collections::HashMap<u32, String> = parser
        .encounter
        .player_data
        .iter()
        .flatten()
        .map(|p| {
            let name = if p.display_name.is_empty() {
                p.character_name.clone()
            } else {
                p.display_name.clone()
            };
            (p.actor_index, name)
        })
        .collect();

    let mut players: Vec<_> = parser.derived_state.party.values().collect();
    players.sort_by(|a, b| b.total_damage.cmp(&a.total_damage));

    let rows = players.into_iter().map(|player| {
        let name = names
            .get(&player.index)
            .cloned()
            .unwrap_or_else(|| format!("{:?}", player.character_type));

        Row::new(vec![
            Cell::from(name),
            Cell::from(format!("{:?}", player.character_type)),
            Cell::from(format!("{}", player.total_damage)),
            Cell::from(format!("{:.0}", player.dps)),
        ])
    });

    let header = Row::new(vec!["Player", "Character", "Damage", "DPS"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(35),
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("GBFR Logs — Meter"));

    frame.render_widget(table, area);
}

fn draw_log(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, log: &SharedLog) {
    let log = log.lock().unwrap();
    let text = log
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Injector log"));

    frame.render_widget(paragraph, area);
}

fn draw_status(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, connected: bool) {
    let status = if connected {
        "Connected to game — press q to quit"
    } else {
        "Waiting for game... — press q to quit"
    };

    frame.render_widget(Paragraph::new(status), area);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let log: SharedLog = Arc::new(Mutex::new(VecDeque::new()));
    spawn_injector(args.appid, args.injector, log.clone());

    let parser = Arc::new(Mutex::new(EngineParser::new(None, None)));
    let connected = Arc::new(AtomicBool::new(false));

    tokio::spawn(run_network_loop(parser.clone(), connected.clone()));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_ui(&mut terminal, &parser, &connected, &log);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}
