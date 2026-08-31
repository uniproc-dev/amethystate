use crate::app::{App, ViewMode};

use amethystate::store::meta::SchemaSnapshot;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let content = match &app.mode {
        ViewMode::Flatten => render_flatten(app),
        ViewMode::All => render_all(app),
        ViewMode::Struct(prefix) => render_struct(app, &prefix.clone()),
    };

    let paragraph =
        Paragraph::new(content).block(Block::default().borders(Borders::ALL).title(" Viewer "));

    f.render_widget(paragraph, area);
}

fn render_flatten(app: &mut App) -> Vec<Line<'static>> {
    match app.backend.scan_all() {
        Ok(entries) => entries
            .into_iter()
            .map(|(k, v)| {
                let val_str = String::from_utf8_lossy(&v).to_string();
                Line::from(vec![
                    Span::styled(k.as_str().to_string(), Style::default().fg(Color::Cyan)),
                    Span::raw(" = "),
                    Span::raw(val_str),
                ])
            })
            .collect(),
        Err(e) => vec![Line::from(Span::styled(
            format!("error: {e}"),
            Style::default().fg(Color::Red),
        ))],
    }
}

fn render_all(app: &mut App) -> Vec<Line<'static>> {
    let snapshots = match app.backend.get_schema_snapshots() {
        Ok(s) => s,
        Err(e) => {
            return vec![Line::from(Span::styled(
                format!("error: {e}"),
                Style::default().fg(Color::Red),
            ))];
        }
    };

    let mut lines = Vec::new();
    for (prefix, snapshot) in snapshots {
        lines.extend(render_snapshot_lines(&prefix, &snapshot, app));
        lines.push(Line::raw(""));
    }
    lines
}

fn render_struct(app: &mut App, prefix: &str) -> Vec<Line<'static>> {
    let snapshots = match app.backend.get_schema_snapshots() {
        Ok(s) => s,
        Err(e) => {
            return vec![Line::from(Span::styled(
                format!("error: {e}"),
                Style::default().fg(Color::Red),
            ))];
        }
    };

    snapshots
        .into_iter()
        .find(|(p, _)| p == prefix)
        .map(|(p, s)| render_snapshot_lines(&p, &s, app))
        .unwrap_or_default()
}

fn render_snapshot_lines(
    prefix: &str,
    snapshot: &SchemaSnapshot,
    app: &mut App,
) -> Vec<Line<'static>> {
    let struct_name = snapshot
        .struct_name
        .clone()
        .unwrap_or_else(|| prefix.to_string());
    let mut lines = vec![Line::from(vec![
        Span::styled(struct_name, Style::default().fg(Color::Yellow)),
        Span::raw(" {"),
    ])];

    for field in &snapshot.fields {
        let path = format!("{}.{}", prefix, field.name);
        let val_str = match app.backend.scan_all() {
            Ok(entries) => entries
                .into_iter()
                .find(|(k, _)| k.as_str() == path)
                .map(|(_, v)| String::from_utf8_lossy(&v).to_string())
                .unwrap_or_else(|| "<missing>".to_string()),
            Err(_) => "<error>".to_string(),
        };

        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(field.name.clone(), Style::default().fg(Color::Cyan)),
            Span::raw(": "),
            Span::styled(
                field.type_name.clone(),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(" = "),
            Span::raw(val_str),
        ]));
    }

    lines.push(Line::raw("}"));
    lines
}
