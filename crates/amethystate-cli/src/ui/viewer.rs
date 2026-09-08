use crate::app::{App, ViewMode};

use amethystate::store::StorePath;
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
        ViewMode::Struct(row) => render_struct(app, *row),
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

fn render_struct(app: &mut App, row: usize) -> Vec<Line<'static>> {
    let Some((prefix, snapshot)) = app.structs.get(row).cloned() else {
        return Vec::new();
    };

    render_snapshot_lines(&prefix, &snapshot, app)
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

    let under = StorePath::parse_joined(prefix).ok();

    for field in &snapshot.fields {
        let at = match &under {
            Some(under) => under.join(&field.name),
            None => field.name.clone(),
        };
        let val_str = match app.backend.scan_all() {
            Ok(entries) => entries
                .into_iter()
                .find(|(k, _)| *k == at)
                .map(|(_, v)| String::from_utf8_lossy(&v).to_string())
                .unwrap_or_else(|| "<missing>".to_string()),
            Err(_) => "<error>".to_string(),
        };

        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(field.name.to_string(), Style::default().fg(Color::Cyan)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use amethystate::amethystate;
    use amethystate::store::builder::{Backend, StoreBuilder};
    use amethystate_core::test_utils::TempPath;

    #[amethystate(prefix = "shared")]
    pub struct Left {
        #[amestate(default = 1u32)]
        pub left: u32,
    }

    #[amethystate(prefix = "shared")]
    pub struct Right {
        #[amestate(default = 2u32)]
        pub right: u32,
    }

    fn named(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_row_opens_the_declaration_recorded_at_it() {
        let held = TempPath::new("viewer_rows");
        let path = held.path().with_extension("json");
        {
            let store = StoreBuilder::new(&path)
                .backend(Backend::Json)
                .build()
                .unwrap();
            Left::new_with(&store).unwrap();
            Right::new_with(&store).unwrap();
            store.save_now().unwrap();
        }

        let backend = crate::inspector::open_inspector(&path).unwrap();
        let mut app = App::new(backend).unwrap();

        let at = app
            .structs
            .iter()
            .position(|(_, held)| held.struct_name.as_deref() == Some("Right"))
            .expect("both declarations sit at `shared` and both are recorded");

        assert!(
            app.structs.iter().filter(|(at, _)| at == "shared").count() == 2,
            "the two rows have to share a prefix, or the row is doing the \
             prefix's job and this states nothing: {:?}",
            app.structs.iter().map(|(at, _)| at).collect::<Vec<_>>()
        );

        assert_eq!(
            named(&render_struct(&mut app, at)[0]),
            "Right {",
            "the row named a prefix rather than a declaration, so it opened \
             whichever was recorded there first"
        );
    }
}
