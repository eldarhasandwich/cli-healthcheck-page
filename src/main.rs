use std::{io, time::Duration};

use crossterm::event::{self, Event, KeyCode};

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use ratatui::{
    backend::CrosstermBackend,
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

struct AppData {
    endpoints: Vec<Endpoint>,
    tick_time_ms: u64,
    gui_refresh_interval_ms: u64,
    history_max_width: usize,
    history_entry_timeframe_ms: u64
}

struct Endpoint {
    name: String,
    url: String,
    expected_code: u16,
    poll_time_ms: u64
}

struct VolatileAppData<'a> {
    since_last_gui_refresh_ms: u64,
    since_last_new_history_timeframe_ms: u64,
    endpoints: Vec<EndpointVolatileData<'a>>
}

struct EndpointVolatileData<'a> {
    endpoint_reference: &'a Endpoint,
    history: Vec<HistoryEntry>,
    time_since_last_poll_ms: u64,
    needs_new_history_entry: bool
}

struct HistoryEntry {
    history_state: HistoryState
}

#[derive(PartialEq)]
enum HistoryState {
    AllSuccesses,
    PartialSuccesses,
    AllFailures
}

async fn smack_endpoint(endpoint: &Endpoint) -> bool {
    return match reqwest::Client::new()
        .get(&endpoint.url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
    {
        Ok(response) => response.status().as_u16() == endpoint.expected_code,
        Err(_) => false,
    };
}

fn write_to_history(app_data: &AppData, endpoint: &mut EndpointVolatileData<'_>, is_success: bool) {

    /*
     * If we are writing new history entry:
     *     - push new entry
     *     - cull earliest entry if we are too long
     *     - return
     */

    if endpoint.needs_new_history_entry {
        if is_success {
            endpoint.history.push(HistoryEntry {
                history_state: HistoryState::AllSuccesses
            });
        } else {
            endpoint.history.push(HistoryEntry {
                history_state: HistoryState::AllFailures
            });
        }

        if endpoint.history.len() > app_data.history_max_width {
            endpoint.history.remove(0);
        }

        endpoint.needs_new_history_entry = false;
        return;
    }

    if let Some(last_entry) = endpoint.history.last_mut() {
        if is_success && last_entry.history_state == HistoryState::AllFailures {
            last_entry.history_state = HistoryState::PartialSuccesses;
        }
    
        if !is_success && last_entry.history_state == HistoryState::AllSuccesses {
            last_entry.history_state = HistoryState::PartialSuccesses;
        }
    }

}

async fn update_endpoints(app_data: &AppData, volatile_app_data: &mut VolatileAppData<'_>) {

    for endpoint in &mut volatile_app_data.endpoints {

        // Skip healthcheck if we have not waited long enough
        endpoint.time_since_last_poll_ms += app_data.tick_time_ms;
        if endpoint.time_since_last_poll_ms < endpoint.endpoint_reference.poll_time_ms {
            continue
        }

        endpoint.time_since_last_poll_ms = 0;

        let is_success: bool = smack_endpoint(endpoint.endpoint_reference).await;

        write_to_history(app_data, endpoint, is_success);
    }
}

fn render_history(history: &[HistoryEntry]) -> String {
    history
        .iter()
        .map(|entry| match entry.history_state {
            HistoryState::AllSuccesses => "█ ",
            HistoryState::PartialSuccesses => "▒ ",
            HistoryState::AllFailures => "░ ",
        })
        .collect()
}

fn draw_ui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    volatile_app_data: &VolatileAppData,
) -> io::Result<()> {
    terminal.draw(|frame| {
        let mut lines: Vec<String> = vec![];

        lines.push("q: quit".to_string());
        lines.push("".to_string());

        for endpoint in &volatile_app_data.endpoints {
            let history = render_history(&endpoint.history);

            lines.push(format!("{:<16} {}", endpoint.endpoint_reference.name, history));
            lines.push(format!("{:<16} {}", "", history));
            lines.push(format!("{:<16} {}", "", history));
            lines.push("".to_string());
        }

        let widget = Paragraph::new(lines.join("\n")).block(
            Block::default()
                .title("cli-healthcheck-page")
                .borders(Borders::ALL),
        );

        frame.render_widget(widget, frame.area());
    })?;

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app_data: AppData,
) -> io::Result<()> {

    let mut volatile_app_data: VolatileAppData<'_> = VolatileAppData { 
        since_last_gui_refresh_ms: 0,
        since_last_new_history_timeframe_ms: 0,
        endpoints: vec![],
    };

    for endpoint in &app_data.endpoints {
        volatile_app_data.endpoints.push(
            EndpointVolatileData {
                endpoint_reference: endpoint,
                history: vec![],
                time_since_last_poll_ms: 0,
                needs_new_history_entry: true
            }
        );
    }

    loop {
        
        // Poll for 'Q' to quit application
        if event::poll(Duration::from_millis(0))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    return Ok(());
                }
            }
        }

        volatile_app_data.since_last_new_history_timeframe_ms += app_data.tick_time_ms;
        if volatile_app_data.since_last_new_history_timeframe_ms >= app_data.history_entry_timeframe_ms {
            
            for endpoint in &mut volatile_app_data.endpoints {
                endpoint.needs_new_history_entry = true;
            }
            volatile_app_data.since_last_new_history_timeframe_ms = 0;
        }

        // Check and run healthchecks
        update_endpoints(&app_data, &mut volatile_app_data).await;

        // Draw interface if it has been long enough
        volatile_app_data.since_last_gui_refresh_ms += app_data.tick_time_ms;
        if volatile_app_data.since_last_gui_refresh_ms >= app_data.gui_refresh_interval_ms {
            draw_ui(terminal, &volatile_app_data)?;
            volatile_app_data.since_last_gui_refresh_ms = 0;
        }

        // Sleep until next tick
        tokio::time::sleep(Duration::from_millis(app_data.tick_time_ms)).await;
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let app_data: AppData = AppData {
        endpoints: vec![
            Endpoint {
                name: "Google".to_string(),
                url: "https://clients3.google.com/generate_204".to_string(),
                expected_code: 204,
                poll_time_ms: 5000,
            },
            Endpoint {
                name: "Cloudflare".to_string(),
                url: "https://1.1.1.1/".to_string(),
                expected_code: 200,
                poll_time_ms: 5000,
            },
        ],
        gui_refresh_interval_ms: 1000,
        tick_time_ms: 10,
        history_max_width: 60,
        history_entry_timeframe_ms: 1000 * 60
    };

    enable_raw_mode()?;

    let mut stdout: io::Stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend: CrosstermBackend<io::Stdout> = CrosstermBackend::new(stdout);
    let mut terminal: Terminal<CrosstermBackend<io::Stdout>> = Terminal::new(backend)?;

    // first draw
    // draw_ui(&mut terminal, &vo)?;

    let result: Result<(), io::Error> = run_app(&mut terminal, app_data).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}