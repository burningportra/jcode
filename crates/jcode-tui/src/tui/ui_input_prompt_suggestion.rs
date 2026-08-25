use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub(crate) fn apply_ghost_style(lines: &mut [Line<'_>], prompt_len: usize, color: Color) {
    for (idx, line) in lines.iter_mut().enumerate() {
        let ghost_start = if idx == 0 {
            prompt_len.min(line.width())
        } else {
            prompt_len.min(line.width())
        };
        let original = line.clone();
        *line = restyle_from_column(original, ghost_start, Style::default().fg(color).dim());
    }
}

fn restyle_from_column<'a>(line: Line<'a>, start_col: usize, style: Style) -> Line<'a> {
    let mut col = 0usize;
    let mut spans = Vec::new();
    for span in line.spans {
        let width = span.width();
        let next = col.saturating_add(width);
        if next <= start_col {
            spans.push(span);
        } else if col >= start_col {
            spans.push(Span::styled(span.content.into_owned(), style));
        } else {
            let text = span.content.into_owned();
            let split = start_col - col;
            let mut left = String::new();
            let mut right = String::new();
            let mut w = 0usize;
            for ch in text.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if w.saturating_add(cw) <= split {
                    left.push(ch);
                    w += cw;
                } else {
                    right.push(ch);
                }
            }
            if !left.is_empty() {
                spans.push(Span::raw(left));
            }
            if !right.is_empty() {
                spans.push(Span::styled(right, style));
            }
        }
        col = next;
    }
    Line::from(spans)
}
