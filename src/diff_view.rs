//! Exibição lado a lado do diff, com números de linha e destaque do Localizar.

use std::ops::Range;

use eframe::egui::{
    self, Align2, Color32, Pos2, Rect, Sense, Stroke, TextStyle, pos2, text::LayoutJob, text::TextFormat, vec2,
};

use rustmerge::diff::{FileDiff, Row, RowKind, inline_segments};
use rustmerge::ops::Side;
use rustmerge::search::find_all;

pub struct Palette {
    pub changed: Color32,
    pub changed_strong: Color32,
    pub removed: Color32,
    pub added: Color32,
    pub filler: Color32,
    pub current: Color32,
    pub line_number: Color32,
    pub text: Color32,
    pub divider: Color32,
    /// Ocorrências do Localizar.
    pub found: Color32,
    /// Ocorrência atual do Localizar.
    pub found_current: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            changed: Color32::from_rgb(70, 60, 25),
            changed_strong: Color32::from_rgb(140, 110, 20),
            removed: Color32::from_rgb(85, 35, 35),
            added: Color32::from_rgb(30, 70, 35),
            filler: Color32::from_gray(45),
            current: Color32::from_rgb(90, 160, 255),
            line_number: Color32::from_gray(120),
            text: Color32::from_gray(220),
            divider: Color32::from_gray(70),
            found: Color32::from_rgb(40, 85, 150),
            found_current: Color32::from_rgb(255, 150, 0),
        }
    } else {
        Palette {
            changed: Color32::from_rgb(255, 245, 190),
            changed_strong: Color32::from_rgb(255, 210, 90),
            removed: Color32::from_rgb(255, 215, 215),
            added: Color32::from_rgb(210, 245, 210),
            filler: Color32::from_gray(225),
            current: Color32::from_rgb(30, 100, 220),
            line_number: Color32::from_gray(140),
            text: Color32::from_gray(20),
            divider: Color32::from_gray(190),
            found: Color32::from_rgb(170, 205, 255),
            found_current: Color32::from_rgb(255, 150, 0),
        }
    }
}

/// Uma ocorrência do Localizar no diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffMatch {
    pub row: usize,
    pub side: Side,
    pub range: Range<usize>,
}

/// Todas as ocorrências num lado do diff, em ordem de linha.
pub fn find_in_diff(diff: &FileDiff, side: Side, query: &str, case_sensitive: bool) -> Vec<DiffMatch> {
    let mut matches = Vec::new();
    for (row_index, row) in diff.rows.iter().enumerate() {
        let line = match side {
            Side::Left => row.left.map(|i| &diff.left.lines[i]),
            Side::Right => row.right.map(|i| &diff.right.lines[i]),
        };
        if let Some(line) = line {
            for range in find_all(line, query, case_sensitive) {
                matches.push(DiffMatch { row: row_index, side, range });
            }
        }
    }
    matches
}

/// Ocorrências a destacar e qual é a atual.
pub struct Highlight<'a> {
    pub matches: &'a [DiffMatch],
    pub current: Option<usize>,
}

/// Desenha o diff. `scroll_to_row` é consumido quando o usuário navega até um bloco ou ocorrência.
pub fn show(
    ui: &mut egui::Ui,
    diff: &FileDiff,
    current_block: Option<usize>,
    scroll_to_row: &mut Option<usize>,
    highlight: &Highlight<'_>,
) {
    let pal = palette(ui.visuals().dark_mode);
    let font = TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.text_style_height(&TextStyle::Monospace);
    let char_width = ui.painter().layout_no_wrap("0".into(), font.clone(), pal.text).size().x;

    // Largura da margem de números conforme a maior linha.
    let max_lines = diff.left.lines.len().max(diff.right.lines.len()).max(1);
    let gutter = (max_lines.to_string().len() as f32 + 1.5) * char_width;

    let current = current_block.and_then(|i| diff.blocks.get(i)).cloned();

    ui.spacing_mut().item_spacing.y = 0.0;
    let mut area = egui::ScrollArea::vertical().id_salt("diff_scroll").auto_shrink([false, false]);
    if let Some(row) = scroll_to_row.take() {
        // Deixa algumas linhas de contexto acima.
        area = area.vertical_scroll_offset(row.saturating_sub(5) as f32 * row_height);
    }

    area.show_rows(ui, row_height, diff.rows.len(), |ui, range| {
        for index in range {
            let row = diff.rows[index];
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::hover());
            let half = rect.width() / 2.0;
            let left_rect = Rect::from_min_size(rect.min, vec2(half, row_height));
            let right_rect = Rect::from_min_max(pos2(rect.left() + half, rect.top()), rect.max);
            let is_current = current.as_ref().is_some_and(|r| r.contains(&index));

            let (left_segs, right_segs) = segments_for(diff, row);
            let found = |side: Side| row_highlights(highlight, index, side);
            let ctx = DrawCtx { gutter, font: &font, pal: &pal };
            draw_side(ui, left_rect, row, Side::Left, left_segs, &found(Side::Left), is_current, &ctx);
            draw_side(ui, right_rect, row, Side::Right, right_segs, &found(Side::Right), is_current, &ctx);
            ui.painter()
                .vline(rect.left() + half, rect.y_range(), Stroke::new(1.0_f32, pal.divider));
        }
    });
}

/// Ocorrências numa linha e lado: (intervalo, é a atual).
fn row_highlights(highlight: &Highlight<'_>, row: usize, side: Side) -> Vec<(Range<usize>, bool)> {
    let matches = highlight.matches;
    let start = matches.partition_point(|m| m.row < row);
    matches[start..]
        .iter()
        .enumerate()
        .take_while(|(_, m)| m.row == row)
        .filter(|(_, m)| m.side == side)
        .map(|(i, m)| (m.range.clone(), highlight.current == Some(start + i)))
        .collect()
}

type Segments = Option<Vec<(String, bool)>>;

/// Texto de cada lado da linha; nas linhas alteradas, já separado em trechos iguais e diferentes.
fn segments_for(diff: &FileDiff, row: Row) -> (Segments, Segments) {
    let left = row.left.map(|i| diff.left.lines[i].as_str());
    let right = row.right.map(|i| diff.right.lines[i].as_str());
    match (row.kind, left, right) {
        (RowKind::Changed, Some(l), Some(r)) => {
            let (l, r) = inline_segments(l, r);
            (Some(l), Some(r))
        }
        _ => (
            left.map(|s| vec![(s.to_string(), false)]),
            right.map(|s| vec![(s.to_string(), false)]),
        ),
    }
}

struct DrawCtx<'a> {
    gutter: f32,
    font: &'a egui::FontId,
    pal: &'a Palette,
}

#[allow(clippy::too_many_arguments)]
fn draw_side(
    ui: &egui::Ui,
    rect: Rect,
    row: Row,
    side: Side,
    segments: Segments,
    found: &[(Range<usize>, bool)],
    is_current: bool,
    ctx: &DrawCtx<'_>,
) {
    let (pal, font, gutter) = (ctx.pal, ctx.font, ctx.gutter);
    let painter = ui.painter_at(rect);
    let line = match side {
        Side::Left => row.left,
        Side::Right => row.right,
    };

    let background = match (row.kind, side, line.is_some()) {
        (RowKind::Equal, _, _) => None,
        (_, _, false) => Some(pal.filler),
        (RowKind::Changed, _, _) => Some(pal.changed),
        (RowKind::LeftOnly, Side::Left, _) => Some(pal.removed),
        (RowKind::RightOnly, Side::Right, _) => Some(pal.added),
        _ => None,
    };
    if let Some(color) = background {
        painter.rect_filled(rect, 0.0, color);
    }
    if is_current {
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, pal.current);
    }

    let Some(line) = line else { return };

    painter.text(
        pos2(rect.left() + gutter - 4.0, rect.center().y),
        Align2::RIGHT_CENTER,
        (line + 1).to_string(),
        font.clone(),
        pal.line_number,
    );

    let mut job = LayoutJob::default();
    append_highlighted(&mut job, &segments.unwrap_or_default(), found, font, pal);
    let galley = painter.layout_job(job);
    painter.galley(Pos2::new(rect.left() + gutter + 4.0, rect.top()), galley, pal.text);
}

/// Monta o texto da linha juntando o destaque das diferenças e o do Localizar
/// (o do Localizar tem prioridade).
fn append_highlighted(
    job: &mut LayoutJob,
    segments: &[(String, bool)],
    found: &[(Range<usize>, bool)],
    font: &egui::FontId,
    pal: &Palette,
) {
    let mut offset = 0;
    for (text, changed) in segments {
        let (seg_start, seg_end) = (offset, offset + text.len());
        let mut cuts = vec![seg_start, seg_end];
        for (r, _) in found {
            cuts.extend([r.start, r.end].into_iter().filter(|&b| b > seg_start && b < seg_end));
        }
        cuts.sort_unstable();
        cuts.dedup();

        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let Some(piece) = text.get(a - seg_start..b - seg_start) else { continue };
            let hit = found.iter().find(|(r, _)| r.start <= a && b <= r.end);
            let (background, color) = match hit {
                Some((_, true)) => (pal.found_current, Color32::BLACK),
                Some((_, false)) => (pal.found, pal.text),
                None if *changed => (pal.changed_strong, pal.text),
                None => (Color32::TRANSPARENT, pal.text),
            };
            let format = TextFormat { font_id: font.clone(), color, background, ..Default::default() };
            job.append(&piece.replace('\t', "    "), 0.0, format);
        }
        offset = seg_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustmerge::text::TextFile;

    #[test]
    fn localiza_so_no_lado_escolhido() {
        let file = |lines: &[&str]| TextFile { lines: lines.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let diff = FileDiff::new("a.txt".into(), file(&["nome", "x"]), file(&["nome nome", "y"]));
        let left = find_in_diff(&diff, Side::Left, "nome", false);
        let right = find_in_diff(&diff, Side::Right, "nome", false);
        assert_eq!(left.len(), 1);
        assert_eq!(right.len(), 2);
        assert!(left.iter().all(|m| m.side == Side::Left));
        assert!(right.iter().all(|m| m.side == Side::Right));
    }
}
