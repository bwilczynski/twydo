use super::{dim, overlay_rect, theme};
use crate::listform::{ListField, ListForm};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph, Wrap},
};

pub fn render(f: &mut Frame, area: Rect, form: &ListForm) {
    let mut lines = vec![Line::styled(format!("ID: {}", form.id), dim())];
    for field in form.fields() {
        let text = match field {
            ListField::Name => format!("Name: {}", form.name),
            ListField::Color => format!("Color: {}  (Left/Right: presets)", form.color),
            ListField::Save => "Save to CalDAV".into(),
            ListField::Archive => if form.original.as_ref().is_some_and(|s| s.archived) {
                "Unarchive on this desktop"
            } else {
                "Archive on this desktop"
            }
            .into(),
            ListField::Delete => "Preview deletion from CalDAV and all devices…".into(),
            ListField::Reconcile => "Check pending save".into(),
            ListField::KeepServer => "Keep server values".into(),
        };
        let selected = field == form.focus;
        lines.push(Line::from(vec![
            Span::raw(if selected { "› " } else { "  " }),
            Span::styled(
                text,
                if selected {
                    theme::accent()
                } else {
                    Default::default()
                },
            ),
        ]));
    }
    if form.operation.is_some() {
        lines.push(Line::raw(format!("Requested name: {}", form.name)));
        lines.push(Line::raw(format!("Requested color: {}", form.color)));
    }
    if let Some((name, color)) = &form.server_values {
        lines.push(Line::raw(format!(
            "Server name: {}",
            name.as_deref().unwrap_or("(none)")
        )));
        lines.push(Line::raw(format!(
            "Server color: {}",
            color.as_deref().unwrap_or("(none)")
        )));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Archive is local; Save applies immediately, including when sync is off.",
        dim(),
    ));
    if form.busy {
        lines.push(Line::styled("Working…", theme::accent()));
    }
    if let Some(notice) = &form.notice {
        lines.push(Line::raw(notice.clone()));
    }
    if let Some(error) = &form.error {
        lines.push(Line::styled(error.clone(), theme::warning()));
    }
    let rect = overlay_rect(
        area,
        76,
        (lines.len() * 2 + 2).min(u16::MAX as usize) as u16,
    );
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered().border_type(BorderType::Rounded).title(
                if form.original.is_none() && form.operation.is_none() {
                    " New list "
                } else {
                    " Manage list "
                },
            ),
        ),
        rect,
    );
}

pub fn backups(f: &mut Frame, area: Rect, panel: &crate::app::CollectionBackups) {
    let mut lines = vec![
        Line::raw("Raw collection snapshots; separate from task undo and :recover."),
        Line::raw("j/k select · Esc close · :export-list-backup <id> <new directory>"),
        Line::raw(""),
    ];
    if panel.backups.is_empty() {
        lines.push(Line::styled("No completed collection backups", dim()));
    }
    for (index, b) in panel.backups.iter().enumerate() {
        lines.push(Line::styled(
            format!(
                "{} {} · {} ({}) · {} remote / {} local files",
                if index == panel.cursor { "›" } else { " " },
                b.backup,
                b.name,
                b.collection,
                b.resources,
                b.local_files
            ),
            if index == panel.cursor {
                theme::accent()
            } else {
                Default::default()
            },
        ));
    }
    lines.push(Line::raw(
        "All raw resources remain exportable. Collection restore awaits lifecycle verification.",
    ));
    let rect = overlay_rect(area, 110, area.height.saturating_sub(4).min(30));
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((
                panel
                    .cursor
                    .saturating_sub(rect.height.saturating_sub(7) as usize) as u16,
                0,
            ))
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .title(" Collection backups "),
            ),
        rect,
    );
}
