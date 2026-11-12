//! Árvore de arquivos virtualizada: só as linhas visíveis na tela são desenhadas,
//! então ela continua leve com centenas de milhares de arquivos.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use eframe::egui::{
    self, Color32, Rect, Sense, Shape, Stroke, TextStyle, pos2,
    text::{LayoutJob, TextFormat, TextWrapping},
    vec2,
};

use rustmerge::folder::{Comparison, FileInfo, Node, Status};
use rustmerge::format::{format_date, format_size};
use rustmerge::ops::Side;

pub fn status_color(status: Status, dark: bool) -> Color32 {
    match (status, dark) {
        (Status::Equal, true) => Color32::from_gray(200),
        (Status::Equal, false) => Color32::from_gray(40),
        (Status::Different, true) => Color32::from_rgb(255, 120, 120),
        (Status::Different, false) => Color32::from_rgb(190, 30, 30),
        (Status::LeftOnly, true) => Color32::from_rgb(130, 170, 255),
        (Status::LeftOnly, false) => Color32::from_rgb(30, 80, 200),
        (Status::RightOnly, true) => Color32::from_rgb(120, 210, 120),
        (Status::RightOnly, false) => Color32::from_rgb(20, 130, 40),
        (Status::Error, _) => Color32::from_rgb(230, 150, 0),
    }
}

/// O que o usuário fez na árvore.
pub enum TreeClick {
    /// Clique simples num arquivo: mostra o diff.
    Select(usize),
    /// "Editar" no menu do botão direito: abre o editor.
    Edit(usize),
    /// Opção do menu do botão direito, aplicada aos itens selecionados.
    Action(Vec<usize>, FileAction),
}

/// Ações do menu do botão direito (valem para arquivos e pastas).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    /// Copia para o lado indicado.
    Copy(Side),
    /// Move para o lado indicado.
    Move(Side),
    /// Apaga de um lado, ou dos dois (`None`).
    Delete(Option<Side>),
    Rename,
}

pub struct TreeState {
    /// Pasta aberta ou fechada, por índice de nó.
    expanded: Vec<bool>,
    /// Nós visíveis, na ordem em que aparecem.
    rows: Vec<usize>,
    rows_only_diffs: Option<bool>,
    /// Itens selecionados (índices de nó).
    selection: BTreeSet<usize>,
    /// Ponto de partida do Shift+clique.
    anchor: Option<usize>,
}

/// Pastas abertas e itens selecionados, pelo caminho, para manter depois de comparar de novo.
pub struct TreeSnapshot {
    expansion: HashMap<PathBuf, bool>,
    selected: HashSet<PathBuf>,
}

/// Clique numa linha, aplicado depois de desenhar todas as linhas.
struct RowClick {
    index: usize,
    secondary: bool,
    modifiers: egui::Modifiers,
}

impl TreeState {
    pub fn new(cmp: &Comparison) -> Self {
        // Começa com as pastas que têm diferenças abertas.
        let expanded = cmp.nodes.iter().map(|n| n.is_dir && n.status != Status::Equal).collect();
        Self { expanded, rows: Vec::new(), rows_only_diffs: None, selection: BTreeSet::new(), anchor: None }
    }

    pub fn snapshot(&self, cmp: &Comparison) -> TreeSnapshot {
        TreeSnapshot {
            expansion: cmp
                .nodes
                .iter()
                .zip(&self.expanded)
                .filter(|(n, _)| n.is_dir)
                .map(|(n, open)| (n.rel_path.clone(), *open))
                .collect(),
            selected: self.selection.iter().map(|&i| cmp.nodes[i].rel_path.clone()).collect(),
        }
    }

    /// Nova árvore mantendo pastas abertas e seleção; pastas novas seguem o padrão.
    pub fn restore(cmp: &Comparison, snapshot: &TreeSnapshot) -> Self {
        let mut state = Self::new(cmp);
        for (i, node) in cmp.nodes.iter().enumerate() {
            if let Some(open) = snapshot.expansion.get(&node.rel_path) {
                state.expanded[i] = *open;
            }
            if snapshot.selected.contains(&node.rel_path) {
                state.selection.insert(i);
            }
        }
        state
    }

    /// Seleciona só este item (por exemplo, ao abrir um arquivo).
    pub fn select_only(&mut self, index: usize) {
        self.selection.clear();
        self.selection.insert(index);
        self.anchor = Some(index);
    }

    /// Itens selecionados, sem os que já estão dentro de uma pasta selecionada
    /// (a operação na pasta já inclui o que está dentro).
    fn selected_items(&self, cmp: &Comparison) -> Vec<usize> {
        self.selection
            .iter()
            .copied()
            .filter(|&i| {
                let mut parent = cmp.nodes[i].parent;
                while let Some(p) = parent {
                    if self.selection.contains(&p) {
                        return false;
                    }
                    parent = cmp.nodes[p].parent;
                }
                true
            })
            .collect()
    }

    /// Aplica o clique: Ctrl marca/desmarca, Shift seleciona o intervalo, clique simples seleciona só um.
    /// Devolve a pasta a abrir/fechar e o arquivo a mostrar, se houver.
    fn apply_click(&mut self, cmp: &Comparison, click: RowClick) -> (Option<usize>, Option<TreeClick>) {
        let index = click.index;
        let is_dir = cmp.nodes[index].is_dir;
        if click.secondary {
            // Botão direito fora da seleção: seleciona só esse item.
            if self.selection.contains(&index) {
                return (None, None);
            }
            self.select_only(index);
            return (None, (!is_dir).then_some(TreeClick::Select(index)));
        }
        let m = click.modifiers;
        if m.shift {
            let anchor = self.anchor.unwrap_or(index);
            let pos = |i: usize| self.rows.iter().position(|&r| r == i);
            if let (Some(a), Some(b)) = (pos(anchor), pos(index)) {
                let range: Vec<usize> = self.rows[a.min(b)..=a.max(b)].to_vec();
                if !m.command {
                    self.selection.clear();
                }
                self.selection.extend(range);
            }
            return (None, None);
        }
        if m.command {
            if !self.selection.remove(&index) {
                self.selection.insert(index);
            }
            self.anchor = Some(index);
            return (None, None);
        }
        self.select_only(index);
        if is_dir { (Some(index), None) } else { (None, Some(TreeClick::Select(index))) }
    }

    /// Recalcula as linhas visíveis no próximo quadro (por exemplo, depois que um status mudou).
    pub fn invalidate(&mut self) {
        self.rows_only_diffs = None;
    }

    fn rebuild(&mut self, cmp: &Comparison, only_diffs: bool) {
        self.rows.clear();
        let mut stack: Vec<usize> = cmp.roots.iter().rev().copied().collect();
        while let Some(index) = stack.pop() {
            let node = &cmp.nodes[index];
            if only_diffs && node.status == Status::Equal {
                continue;
            }
            self.rows.push(index);
            if node.is_dir && self.expanded[index] {
                stack.extend(node.children.iter().rev());
            }
        }
        self.rows_only_diffs = Some(only_diffs);
        // Itens que deixaram de aparecer (pasta fechada, filtro) saem da seleção,
        // para nenhuma operação atingir algo que não está na tela.
        let visible: HashSet<usize> = self.rows.iter().copied().collect();
        self.selection.retain(|i| visible.contains(i));
    }

    /// Desenha a árvore e devolve o clique num arquivo, se houver.
    /// `columns` são as larguras das colunas (ajustáveis arrastando o cabeçalho).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        cmp: &Comparison,
        only_diffs: bool,
        selected: Option<usize>,
        columns: &mut [f32; COLUMN_COUNT],
    ) -> Option<TreeClick> {
        if self.rows_only_diffs != Some(only_diffs) {
            self.rebuild(cmp, only_diffs);
        }

        let dark = ui.visuals().dark_mode;
        let font = TextStyle::Body.resolve(ui.style());
        let row_height = ui.spacing().interact_size.y;
        let text_color = ui.visuals().text_color();
        let indent = 16.0;
        let mut clicked = None;
        let mut row_click = None;

        ui.spacing_mut().item_spacing.y = 0.0;
        // A largura do nome é definida pelo cabeçalho e reaproveitada nas linhas,
        // para as colunas ficarem alinhadas mesmo com a barra de rolagem.
        let name_width = header(ui, row_height, &font, columns);

        egui::ScrollArea::vertical()
            .id_salt("tree_scroll")
            .auto_shrink([false, false])
            .show_rows(ui, row_height, self.rows.len(), |ui, range| {
                for row in range {
                    let index = self.rows[row];
                    let node = &cmp.nodes[index];
                    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
                    let painter = ui.painter_at(rect);

                    // Destaca a seleção (ou o arquivo aberto, se nada estiver selecionado).
                    let highlighted = self.selection.contains(&index) || (self.selection.is_empty() && selected == Some(index));
                    if highlighted {
                        painter.rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
                    } else if response.hovered() {
                        painter.rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
                    }

                    let cells = column_rects(rect, name_width, columns);
                    let color = status_color(node.status, dark);

                    // Nome, com a seta das pastas e o recuo do nível.
                    let x = rect.left() + 4.0 + node.depth as f32 * indent;
                    if node.is_dir {
                        draw_arrow(&painter, pos2(x + 5.0, rect.center().y), self.expanded[index], color);
                    }
                    let name_rect = Rect::from_min_max(pos2(x + 10.0, rect.top()), pos2(cells[0].left(), rect.bottom()));
                    cell_text(&painter, name_rect, &node.name, &font, color, false);

                    // Situação, tamanhos e datas.
                    cell_text(&painter, cells[0], node.status.description(), &font, color, false);
                    let size = |info: Option<FileInfo>| info.map(|i| format_size(i.size)).unwrap_or_default();
                    let date = |info: Option<FileInfo>| info.and_then(|i| i.modified).map(format_date).unwrap_or_default();
                    cell_text(&painter, cells[1], &size(node.left_info), &font, text_color, true);
                    cell_text(&painter, cells[2], &size(node.right_info), &font, text_color, true);
                    cell_text(&painter, cells[3], &date(node.left_info), &font, text_color, false);
                    cell_text(&painter, cells[4], &date(node.right_info), &font, text_color, false);

                    response.context_menu(|ui| {
                        // O menu vale para a seleção se o item fizer parte dela; senão, só para ele.
                        let items =
                            if self.selection.contains(&index) { self.selected_items(cmp) } else { vec![index] };
                        if let Some(click) = context_menu(ui, cmp, items) {
                            clicked = Some(click);
                        }
                    });
                    if response.clicked() || response.secondary_clicked() {
                        row_click = Some(RowClick {
                            index,
                            secondary: response.secondary_clicked(),
                            modifiers: ui.input(|i| i.modifiers),
                        });
                    }
                    response.on_hover_text(format!(
                        "{}\n{} · Ctrl/Shift+clique seleciona vários · botão direito para mais opções",
                        node.name,
                        node.status.description()
                    ));
                }
            });

        if let Some(click) = row_click {
            let (toggle, select) = self.apply_click(cmp, click);
            if let Some(index) = toggle {
                self.expanded[index] = !self.expanded[index];
                self.rebuild(cmp, only_diffs);
            }
            if clicked.is_none() {
                clicked = select;
            }
            ui.ctx().request_repaint();
        }
        clicked
    }
}

pub const COLUMN_COUNT: usize = 5;
const COLUMN_TITLES: [&str; COLUMN_COUNT] = ["Situação", "Tam. esq.", "Tam. dir.", "Data esq.", "Data dir."];
/// Larguras padrão de Situação, tamanhos e datas.
pub const DEFAULT_COLUMNS: [f32; COLUMN_COUNT] = [105.0, 75.0, 75.0, 120.0, 120.0];
const MIN_NAME_WIDTH: f32 = 120.0;

/// Retângulos de cada coluna (depois do nome) dentro de uma linha.
fn column_rects(row: Rect, name_width: f32, columns: &[f32; COLUMN_COUNT]) -> [Rect; COLUMN_COUNT] {
    let mut x = row.left() + name_width;
    columns.map(|w| {
        let r = Rect::from_min_max(pos2(x, row.top()), pos2(x + w, row.bottom()));
        x += w;
        r
    })
}

/// Cabeçalho com os títulos; arrastar a divisória à esquerda de uma coluna muda a largura dela.
/// Devolve a largura da coluna do nome.
fn header(ui: &mut egui::Ui, height: f32, font: &egui::FontId, columns: &mut [f32; COLUMN_COUNT]) -> f32 {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let name_width = (rect.width() - columns.iter().sum::<f32>()).max(MIN_NAME_WIDTH);
    let painter = ui.painter_at(rect);
    let title_color = ui.visuals().strong_text_color();
    let line = Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color);

    let name_rect = Rect::from_min_max(rect.min, pos2(rect.left() + name_width, rect.bottom()));
    cell_text(&painter, name_rect, "Nome", font, title_color, false);
    let cells = column_rects(rect, name_width, columns);
    for (i, cell) in cells.iter().enumerate() {
        cell_text(&painter, *cell, COLUMN_TITLES[i], font, title_color, false);

        let divider = Rect::from_x_y_ranges(cell.left() - 3.0..=cell.left() + 3.0, rect.y_range());
        let response = ui
            .interact(divider, ui.id().with(("tree_column", i)), Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if response.dragged() {
            // Arrastar para a esquerda aumenta a coluna.
            columns[i] = (columns[i] - response.drag_delta().x).clamp(40.0, 400.0);
        }
        painter.vline(cell.left(), rect.y_range(), line);
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, line);
    name_width
}

/// Texto numa célula, numa linha só; se não couber, termina com "…".
fn cell_text(painter: &egui::Painter, rect: Rect, text: &str, font: &egui::FontId, color: Color32, align_right: bool) {
    let width = rect.width() - 8.0;
    if text.is_empty() || width <= 0.0 {
        return;
    }
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font.clone(), color));
    job.wrap = TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let y = rect.center().y - galley.size().y / 2.0;
    let x = if align_right { rect.right() - 4.0 - galley.size().x } else { rect.left() + 4.0 };
    painter.galley(pos2(x, y), galley, color);
}

/// Menu do botão direito. Cada opção só fica ativa se fizer sentido para o item.
/// Cada opção fica ativa se valer para pelo menos um dos itens; "Editar" e "Renomear", só com um item.
fn context_menu(ui: &mut egui::Ui, cmp: &Comparison, items: Vec<usize>) -> Option<TreeClick> {
    let nodes: Vec<&Node> = items.iter().map(|&i| &cmp.nodes[i]).collect();
    let any_left = nodes.iter().any(|n| n.in_left);
    let any_right = nodes.iter().any(|n| n.in_right);
    let any_both = nodes.iter().any(|n| n.in_left && n.in_right);
    let single = items.len() == 1;
    let mut result = None;

    if !single {
        ui.weak(format!("{} itens selecionados", items.len()));
        ui.separator();
    }
    let single_file = single && !nodes[0].is_dir;
    if ui.add_enabled(single_file, egui::Button::new("Editar")).clicked() {
        result = Some(TreeClick::Edit(items[0]));
        ui.close();
    }
    ui.separator();

    let mut item = |ui: &mut egui::Ui, enabled: bool, text: &str, a: FileAction| {
        if ui.add_enabled(enabled, egui::Button::new(text)).clicked() {
            result = Some(TreeClick::Action(items.clone(), a));
            ui.close();
        }
    };
    item(ui, any_left, "Copiar da esquerda para a direita", FileAction::Copy(Side::Right));
    item(ui, any_right, "Copiar da direita para a esquerda", FileAction::Copy(Side::Left));
    ui.separator();
    item(ui, any_left, "Mover da esquerda para a direita", FileAction::Move(Side::Right));
    item(ui, any_right, "Mover da direita para a esquerda", FileAction::Move(Side::Left));
    ui.separator();
    item(ui, any_left, "Apagar da esquerda", FileAction::Delete(Some(Side::Left)));
    item(ui, any_right, "Apagar da direita", FileAction::Delete(Some(Side::Right)));
    item(ui, any_both, "Apagar dos dois lados", FileAction::Delete(None));
    ui.separator();
    item(ui, single, "Renomear...", FileAction::Rename);
    result
}

fn draw_arrow(painter: &egui::Painter, center: egui::Pos2, open: bool, color: Color32) {
    let s = 4.0;
    let points = if open {
        vec![pos2(center.x - s, center.y - s / 2.0), pos2(center.x + s, center.y - s / 2.0), pos2(center.x, center.y + s)]
    } else {
        vec![pos2(center.x - s / 2.0, center.y - s), pos2(center.x + s, center.y), pos2(center.x - s / 2.0, center.y + s)]
    };
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustmerge::folder::{Options, Progress, compare_folders};
    use std::fs;
    use std::path::Path;

    fn sample(name: &str) -> (PathBuf, Comparison) {
        let root = std::env::temp_dir().join(format!("rustmerge_tree_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (l, r) = (root.join("l"), root.join("r"));
        fs::create_dir_all(l.join("pasta")).unwrap();
        fs::create_dir_all(&r).unwrap();
        for f in ["a.txt", "b.txt", "c.txt", "pasta/d.txt"] {
            fs::write(l.join(f), f).unwrap();
        }
        let cmp = compare_folders(&l, &r, Options::default(), &Progress::default()).unwrap();
        (root, cmp)
    }

    fn click(index: usize, ctrl: bool, shift: bool) -> RowClick {
        let modifiers = egui::Modifiers { command: ctrl, ctrl, shift, ..Default::default() };
        RowClick { index, secondary: false, modifiers }
    }

    #[test]
    fn seleciona_com_ctrl_e_shift() {
        let (root, cmp) = sample("select");
        let mut tree = TreeState::new(&cmp);
        tree.rebuild(&cmp, false);
        let idx = |p: &str| cmp.find(Path::new(p)).unwrap();
        let (a, b, c) = (idx("a.txt"), idx("b.txt"), idx("c.txt"));

        // Clique simples: só um, e abre o diff.
        let (_, open) = tree.apply_click(&cmp, click(a, false, false));
        assert!(matches!(open, Some(TreeClick::Select(i)) if i == a));
        assert_eq!(tree.selected_items(&cmp), vec![a]);

        // Ctrl+clique adiciona e remove.
        tree.apply_click(&cmp, click(c, true, false));
        assert_eq!(tree.selected_items(&cmp), vec![a, c]);
        tree.apply_click(&cmp, click(c, true, false));
        assert_eq!(tree.selected_items(&cmp), vec![a]);

        // Shift+clique seleciona o intervalo desde o último clique (a) até c.
        tree.apply_click(&cmp, click(a, false, false));
        tree.apply_click(&cmp, click(c, false, true));
        assert_eq!(tree.selected_items(&cmp), vec![a, b, c]);

        // Pasta e arquivo dentro dela: a operação vale só para a pasta.
        let (pasta, d) = (idx("pasta"), idx("pasta/d.txt"));
        tree.select_only(pasta);
        tree.apply_click(&cmp, click(d, true, false));
        assert_eq!(tree.selected_items(&cmp), vec![pasta]);

        // Botão direito dentro da seleção mantém a seleção.
        tree.apply_click(&cmp, RowClick { index: d, secondary: true, modifiers: Default::default() });
        assert_eq!(tree.selection.len(), 2);

        fs::remove_dir_all(&root).unwrap();
    }
}
