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
    refresh_interval_ms: u64,
    history_max_width: usize,
}

struct Endpoint {
    name: String,
    url: String,
    expected_code: u16,
    history: Vec<bool>,
}

async fn smack_endpoint(endpoint: &Endpoint) -> bool {
    return match reqwest::get(&endpoint.url).await {
        Ok(response) => response.status().as_u16() == endpoint.expected_code,
        Err(_) => false,
    };
}

async fn update_endpoints(app_data: &mut AppData) {
    for endpoint in &mut app_data.endpoints {
        let is_success = smack_endpoint(endpoint).await;

        endpoint.history.push(is_success);

        if endpoint.history.len() > app_data.history_max_width {
            endpoint.history.remove(0);
        }
    }
}

fn render_history(history: &[bool]) -> String {
    history
        .iter()
        .map(|is_success| if *is_success { "█" } else { "░" })
        .collect()
}

fn draw_ui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app_data: &AppData,
) -> io::Result<()> {
    terminal.draw(|frame| {
        let mut lines = vec![];

        lines.push("q: quit".to_string());
        lines.push("".to_string());

        for endpoint in &app_data.endpoints {
            let history = render_history(&endpoint.history);
            lines.push(format!("{:<16} {}", endpoint.name, history));
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
    app_data: &mut AppData,
) -> io::Result<()> {
    loop {

        if event::poll(Duration::from_millis(0))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    return Ok(());
                }
            }
        }

        update_endpoints(app_data).await;
        draw_ui(terminal, app_data)?;

        tokio::time::sleep(Duration::from_millis(app_data.refresh_interval_ms)).await;
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let mut app_data: AppData = AppData {
        endpoints: vec![
            Endpoint {
                name: "Google".to_string(),
                url: "https://clients3.google.com/generate_204".to_string(),
                expected_code: 204,
                history: vec![],
            },
            Endpoint {
                name: "Cloudflare".to_string(),
                url: "https://1.1.1.1/".to_string(),
                expected_code: 200,
                history: vec![],
            },
        ],
        refresh_interval_ms: 1000,
        history_max_width: 24,
    };

    enable_raw_mode()?;

    let mut stdout: io::Stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend: CrosstermBackend<io::Stdout> = CrosstermBackend::new(stdout);
    let mut terminal: Terminal<CrosstermBackend<io::Stdout>> = Terminal::new(backend)?;

    let result: Result<(), io::Error> = run_app(&mut terminal, &mut app_data).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}