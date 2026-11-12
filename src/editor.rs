//! Janela de edição: os dois arquivos lado a lado, editáveis, com as linhas diferentes destacadas.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, Key, KeyboardShortcut, Modifiers, Rect, Shape, TextBuffer, TextStyle,
    ViewportBuilder, ViewportClass, ViewportCommand, ViewportId, pos2,
    text::{CCursor, CCursorRange, LayoutJob, TextFormat},
    vec2,
};

use rustmerge::diff::{RowKind, line_marks};
use rustmerge::ops::Side;
use rustmerge::search::find_all;
use rustmerge::text::{TextEncoding, read_text, write_text};

use crate::diff_view::palette;
use crate::find_bar::{self, FindBar, FindEvent};

struct EditorSide {
    path: PathBuf,
    /// O arquivo existe no disco (se não existir, "Salvar" cria o arquivo).
    exists: bool,
    text: String,
    saved_text: String,
    encoding: TextEncoding,
    crlf: bool,
}

impl EditorSide {
    fn load(path: PathBuf) -> Result<Option<Self>, String> {
        if !path.is_file() {
            return Ok(None);
        }
        let decoded = read_text(&path)
            .map_err(|e| format!("Erro ao ler {}: {e}", path.display()))?
            .ok_or_else(|| format!("{} é um arquivo binário e não pode ser editado.", path.display()))?;
        Ok(Some(Self {
            path,
            exists: true,
            saved_text: decoded.text.clone(),
            text: decoded.text,
            encoding: decoded.encoding,
            crlf: decoded.crlf,
        }))
    }

    /// Lado onde o arquivo ainda não existe: usa a codificação e a quebra de linha do outro lado.
    fn missing(path: PathBuf, other: Option<&EditorSide>) -> Self {
        Self {
            path,
            exists: false,
            text: String::new(),
            saved_text: String::new(),
            encoding: other.map_or(TextEncoding::Utf8, |o| o.encoding),
            crlf: other.map_or(cfg!(windows), |o| o.crlf),
        }
    }

    fn is_dirty(&self) -> bool {
        self.text != self.saved_text
    }

    fn save(&mut self) -> Result<(), String> {
        write_text(&self.path, &self.text, self.encoding, self.crlf)?;
        self.saved_text = self.text.clone();
        self.exists = true;
        Ok(())
    }

    fn lines(&self) -> Vec<String> {
        self.text.split('\n').map(str::to_string).collect()
    }
}

pub struct EditorWindow {
    pub id: ViewportId,
    pub rel_path: PathBuf,
    pub left_root: PathBuf,
    pub right_root: PathBuf,
    left: EditorSide,
    right: EditorSide,
    left_marks: Vec<RowKind>,
    right_marks: Vec<RowKind>,
    marks_dirty: bool,
    confirm_close: bool,
    /// A janela pode ser removida; o app principal então recompara o arquivo.
    pub closed: bool,
    message: Option<(String, bool)>,
    /// Localizar nos dois lados.
    find: FindBar,
    find_matches: Vec<(Side, Range<usize>)>,
    find_current: Option<usize>,
    /// Rolar até esta posição no próximo quadro.
    scroll_to: Option<(Side, usize)>,
    /// Selecionar este trecho no próximo quadro.
    select: Option<(Side, Range<usize>)>,
}

impl EditorWindow {
    pub fn viewport_id(rel_path: &Path) -> ViewportId {
        ViewportId::from_hash_of(("editor", rel_path))
    }

    pub fn open(left_root: &Path, right_root: &Path, rel_path: &Path) -> Result<Self, String> {
        let left_path = left_root.join(rel_path);
        let right_path = right_root.join(rel_path);
        let left = EditorSide::load(left_path.clone())?;
        let right = EditorSide::load(right_path.clone())?;
        if left.is_none() && right.is_none() {
            return Err(format!("{} não existe em nenhum dos lados.", rel_path.display()));
        }
        let left_side = match left {
            Some(l) => l,
            None => EditorSide::missing(left_path, right.as_ref()),
        };
        let right_side = match right {
            Some(r) => r,
            None => EditorSide::missing(right_path, Some(&left_side)),
        };
        Ok(Self {
            id: Self::viewport_id(rel_path),
            rel_path: rel_path.to_path_buf(),
            left_root: left_root.to_path_buf(),
            right_root: right_root.to_path_buf(),
            left: left_side,
            right: right_side,
            left_marks: Vec::new(),
            right_marks: Vec::new(),
            marks_dirty: true,
            confirm_close: false,
            closed: false,
            message: None,
            find: FindBar::default(),
            find_matches: Vec::new(),
            find_current: None,
            scroll_to: None,
            select: None,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.left.is_dirty() || self.right.is_dirty()
    }

    /// Pede para fechar: com alterações não salvas, pergunta antes.
    pub fn request_close(&mut self) {
        if self.is_dirty() {
            self.confirm_close = true;
        } else {
            self.closed = true;
        }
    }

    fn side_mut(&mut self, side: Side) -> &mut EditorSide {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    fn save(&mut self, side: Side) -> bool {
        let result = self.side_mut(side).save();
        let path = self.side_mut(side).path.display().to_string();
        match result {
            Ok(()) => {
                self.message = Some((format!("Salvo: {path}"), false));
                true
            }
            Err(e) => {
                self.message = Some((e, true));
                false
            }
        }
    }

    fn save_all(&mut self) -> bool {
        let mut ok = true;
        for side in [Side::Left, Side::Right] {
            if self.side_mut(side).is_dirty() {
                ok &= self.save(side);
            }
        }
        ok
    }

    /// Mostra a janela. Deve ser chamado a cada quadro enquanto a janela estiver aberta.
    pub fn show(&mut self, ctx: &egui::Context) {
        let title = format!("{}{} — RustMerge", if self.is_dirty() { "● " } else { "" }, self.rel_path.display());
        let builder = ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([600.0, 300.0]);

        ctx.show_viewport_immediate(self.id, builder, |ctx, class| {
            if class == ViewportClass::Embedded {
                // Sistema sem suporte a várias janelas: mostra dentro da janela principal.
                let mut open = true;
                egui::Window::new(title.clone())
                    .open(&mut open)
                    .default_size([1000.0, 600.0])
                    .show(ctx, |ui| self.contents(ui));
                if !open {
                    self.request_close();
                }
                return;
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                if self.is_dirty() {
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                }
                self.request_close();
            }
            egui::CentralPanel::default().show(ctx, |ui| self.contents(ui));
        });
    }

    fn contents(&mut self, ui: &mut egui::Ui) {
        if ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S))) {
            self.save_all();
        }
        let mut find_event = self.find.handle_shortcuts(ui.ctx());
        if self.marks_dirty {
            (self.left_marks, self.right_marks) = line_marks(&self.left.lines(), &self.right.lines());
            self.marks_dirty = false;
        }

        self.toolbar(ui);
        if self.confirm_close {
            self.close_prompt(ui);
        }
        if self.find.open {
            let status = find_bar::status(self.find_current, self.find_matches.len());
            let event = self.find.ui(ui, &status);
            if event != FindEvent::None {
                find_event = event;
            }
        }
        self.handle_find_event(find_event);
        ui.separator();

        let left_view = self.side_view(Side::Left);
        let right_view = self.side_view(Side::Right);
        self.scroll_to = None;
        self.select = None;

        let mut changed = false;
        ui.columns(2, |cols| {
            changed |= side_editor(&mut cols[0], &mut self.left, &self.left_marks, Side::Left, &left_view);
            changed |= side_editor(&mut cols[1], &mut self.right, &self.right_marks, Side::Right, &right_view);
        });
        if changed {
            self.marks_dirty = true;
            self.message = None;
            // O texto mudou: procura de novo, sem pular para outra ocorrência.
            if self.find.open {
                let current = self.find_current;
                self.update_find_matches();
                self.find_current = current.filter(|i| *i < self.find_matches.len()).or(self.find_current);
            }
        }
    }

    /// O que cada lado precisa saber do Localizar neste quadro.
    fn side_view(&self, side: Side) -> SideView {
        let found = if self.find.open {
            self.find_matches
                .iter()
                .enumerate()
                .filter(|(_, (s, _))| *s == side)
                .map(|(i, (_, r))| (r.clone(), self.find_current == Some(i)))
                .collect()
        } else {
            Vec::new()
        };
        SideView {
            found,
            scroll_to: self.scroll_to.as_ref().filter(|(s, _)| *s == side).map(|(_, offset)| *offset),
            select: self.select.as_ref().filter(|(s, _)| *s == side).map(|(_, r)| r.clone()),
        }
    }

    fn update_find_matches(&mut self) {
        // Busca só no lado escolhido na barra.
        let side = self.find.side;
        let text = match side {
            Side::Left => &self.left.text,
            Side::Right => &self.right.text,
        };
        self.find_matches =
            find_all(text, &self.find.query, self.find.case_sensitive).into_iter().map(|r| (side, r)).collect();
        self.find_current = if self.find_matches.is_empty() { None } else { Some(0) };
    }

    fn handle_find_event(&mut self, event: FindEvent) {
        match event {
            FindEvent::Changed => {
                self.update_find_matches();
                self.jump_to_current();
            }
            FindEvent::Next | FindEvent::Prev => {
                self.find_current = find_bar::step(self.find_current, self.find_matches.len(), event == FindEvent::Next);
                self.jump_to_current();
            }
            // Ao fechar a barra, deixa a ocorrência atual selecionada no editor (dá para digitar por cima).
            FindEvent::Closed => self.select = self.find_current.and_then(|i| self.find_matches.get(i)).cloned(),
            FindEvent::None => {}
        }
    }

    fn jump_to_current(&mut self) {
        if let Some((side, range)) = self.find_current.and_then(|i| self.find_matches.get(i)) {
            self.scroll_to = Some((*side, range.start));
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong(self.rel_path.display().to_string());
            ui.separator();
            if ui.add_enabled(self.left.is_dirty(), egui::Button::new("Salvar esquerda")).clicked() {
                self.save(Side::Left);
            }
            if ui.add_enabled(self.right.is_dirty(), egui::Button::new("Salvar direita")).clicked() {
                self.save(Side::Right);
            }
            if ui.add_enabled(self.is_dirty(), egui::Button::new("Salvar tudo (Ctrl+S)")).clicked() {
                self.save_all();
            }
            if ui.button("Localizar (Ctrl+F)").clicked() {
                self.find.show_bar();
            }
            if ui.button("Fechar").clicked() {
                self.request_close();
            }
            let differences = self.left_marks.iter().filter(|k| **k != RowKind::Equal).count()
                + self.right_marks.iter().filter(|k| **k == RowKind::RightOnly).count();
            ui.separator();
            ui.label(if differences == 0 { "Sem diferenças".to_string() } else { format!("{differences} linhas diferentes") });
            if let Some((msg, is_error)) = &self.message {
                ui.separator();
                let color = if *is_error { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() };
                ui.colored_label(color, msg);
            }
        });
    }

    fn close_prompt(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(ui.visuals().warn_fg_color.gamma_multiply(0.15))
            .inner_margin(6.0)
            .corner_radius(4.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Há alterações não salvas.");
                    if ui.button("Salvar e fechar").clicked() && self.save_all() {
                        self.closed = true;
                    }
                    if ui.button("Descartar e fechar").clicked() {
                        self.closed = true;
                    }
                    if ui.button("Cancelar").clicked() {
                        self.confirm_close = false;
                    }
                });
            });
    }
}

/// Localizar, para um lado do editor.
struct SideView {
    /// Ocorrências neste lado: (intervalo de bytes, é a atual).
    found: Vec<(Range<usize>, bool)>,
    /// Rolar até esta posição (em bytes).
    scroll_to: Option<usize>,
    /// Selecionar este trecho e dar foco ao editor.
    select: Option<Range<usize>>,
}

/// Posição em caracteres a partir da posição em bytes (o cursor do egui conta caracteres).
fn char_index(text: &str, byte: usize) -> Option<usize> {
    (byte <= text.len() && text.is_char_boundary(byte)).then(|| text[..byte].chars().count())
}

/// Um lado do editor: cabeçalho, números de linha e texto editável. Devolve true se o texto mudou.
fn side_editor(ui: &mut egui::Ui, side: &mut EditorSide, marks: &[RowKind], which: Side, view: &SideView) -> bool {
    let pal = palette(ui.visuals().dark_mode);
    let font = TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.text_style_height(&TextStyle::Monospace);

    ui.add(egui::Label::new(side_header(side, which)).truncate());

    let line_count = side.text.split('\n').count();
    let char_width = ui.painter().layout_no_wrap("0".into(), font.clone(), pal.text).size().x;
    let gutter = (line_count.to_string().len() as f32 + 1.0) * char_width + 6.0;

    let mut changed = false;
    egui::ScrollArea::both()
        .id_salt(which == Side::Left)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(gutter);
                // Reserva o lugar do fundo colorido das linhas, desenhado depois de saber a posição de cada linha.
                let background = ui.painter().add(Shape::Noop);

                let text_color = pal.text;
                let layout_font = font.clone();
                let found = view.found.clone();
                let (found_bg, found_current_bg) = (pal.found, pal.found_current);
                let mut layouter = move |ui: &egui::Ui, buf: &dyn TextBuffer, _wrap: f32| -> Arc<egui::Galley> {
                    // Sem quebra automática (o padrão do LayoutJob): cada linha do arquivo é uma linha na tela.
                    let text = buf.as_str();
                    let mut job = LayoutJob::default();
                    let plain = TextFormat { font_id: layout_font.clone(), color: text_color, ..Default::default() };
                    let mut pos = 0;
                    for (range, current) in &found {
                        // Ignora ocorrências que ficaram inválidas enquanto o texto é editado.
                        if range.start < pos || char_index(text, range.start).is_none() || char_index(text, range.end).is_none() {
                            continue;
                        }
                        job.append(&text[pos..range.start], 0.0, plain.clone());
                        let (background, color) =
                            if *current { (found_current_bg, Color32::BLACK) } else { (found_bg, text_color) };
                        job.append(&text[range.clone()], 0.0, TextFormat { background, color, ..plain.clone() });
                        pos = range.end;
                    }
                    job.append(&text[pos..], 0.0, plain);
                    ui.painter().layout_job(job)
                };

                // Selecionar a ocorrência (ao fechar o Localizar): ajusta o cursor antes de desenhar.
                let edit_id = ui.make_persistent_id(("editor_text", which == Side::Left));
                let mut select_now = false;
                if let Some(range) = &view.select
                    && let (Some(a), Some(b)) = (char_index(&side.text, range.start), char_index(&side.text, range.end))
                {
                    let mut state = egui::TextEdit::load_state(ui.ctx(), edit_id).unwrap_or_default();
                    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
                    egui::TextEdit::store_state(ui.ctx(), edit_id, state);
                    select_now = true;
                }

                let rows_to_fill = (ui.available_height() / row_height) as usize;
                let output = egui::TextEdit::multiline(&mut side.text)
                    .id(edit_id)
                    .font(font.clone())
                    .code_editor()
                    .lock_focus(true)
                    .frame(false)
                    .margin(egui::Margin::symmetric(4, 0))
                    .desired_width(ui.available_width())
                    .desired_rows(rows_to_fill.max(1))
                    .layouter(&mut layouter)
                    .show(ui);
                changed = output.response.changed();
                if select_now {
                    output.response.request_focus();
                }

                let origin = output.galley_pos;
                // Rola até a ocorrência atual do Localizar.
                if let Some(chars) = view.scroll_to.and_then(|b| char_index(&side.text, b)) {
                    let target = output.galley.pos_from_cursor(CCursor::new(chars)).translate(origin.to_vec2());
                    ui.scroll_to_rect(target.expand2(vec2(40.0, 0.0)), Some(Align::Center));
                }
                let left = origin.x - gutter - 4.0;
                let right = output.response.rect.right().max(ui.clip_rect().right());
                let clip = ui.clip_rect();
                let mut shapes = Vec::new();
                for (i, row) in output.galley.rows.iter().enumerate() {
                    let r = row.rect().translate(origin.to_vec2());
                    if r.bottom() < clip.top() || r.top() > clip.bottom() {
                        continue;
                    }
                    let fill = match (marks.get(i), which) {
                        (Some(RowKind::Changed), _) => Some(pal.changed),
                        (Some(RowKind::LeftOnly), Side::Left) => Some(pal.removed),
                        (Some(RowKind::RightOnly), Side::Right) => Some(pal.added),
                        _ => None,
                    };
                    if let Some(fill) = fill {
                        let line_rect = Rect::from_min_max(pos2(left, r.top()), pos2(right, r.bottom()));
                        shapes.push(Shape::rect_filled(line_rect, 0.0, fill));
                    }
                    ui.painter().text(
                        pos2(origin.x - 8.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        (i + 1).to_string(),
                        font.clone(),
                        pal.line_number,
                    );
                }
                ui.painter().set(background, Shape::Vec(shapes));
            });
        });
    changed
}

/// Cabeçalho de cada lado: caminho, codificação, quebra de linha e se há alterações.
fn side_header(side: &EditorSide, which: Side) -> egui::RichText {
    let label = if which == Side::Left { "Esquerda" } else { "Direita" };
    let state = if !side.exists { " — novo (será criado ao salvar)" } else { "" };
    let dirty = if side.is_dirty() { " ●" } else { "" };
    let eol = if side.crlf { "CRLF" } else { "LF" };
    egui::RichText::new(format!(
        "{label}: {}{state} · {} · {eol}{dirty}",
        side.path.display(),
        side.encoding.name()
    ))
    .color(if side.is_dirty() { Color32::from_rgb(230, 150, 0) } else { Color32::GRAY })
}
