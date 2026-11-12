//! Estado da aplicação e layout das janelas.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use eframe::egui::{self, Key, RichText};

use rustmerge::diff::FileDiff;
use rustmerge::filter::ExtFilter;
use rustmerge::folder::{self, CompareMethod, Comparison, Options, Progress, Status, compare_folders};

use crate::actions::{ModalResult, PendingOp};
use crate::diff_view::{self, DiffMatch, Highlight, find_in_diff};
use crate::find_bar::{self, FindBar, FindEvent};
use crate::editor::EditorWindow;
use crate::settings::Settings;
use crate::tree_view::{self, TreeClick, TreeSnapshot, TreeState};

/// Comparação de pastas rodando em outra thread.
struct ScanJob {
    rx: Receiver<Result<Comparison, String>>,
    progress: Arc<Progress>,
}

/// Arquivo sendo lido e comparado em outra thread.
struct DiffJob {
    rx: Receiver<Result<FileDiff, String>>,
    rel_path: PathBuf,
}

pub struct RustMergeApp {
    settings: Settings,
    comparison: Option<Comparison>,
    tree: Option<TreeState>,
    scan: Option<ScanJob>,
    selected: Option<PathBuf>,
    selected_index: Option<usize>,
    diff_job: Option<DiffJob>,
    file_diff: Option<FileDiff>,
    current_block: Option<usize>,
    scroll_to_row: Option<usize>,
    error: Option<String>,
    /// Janelas de edição abertas (uma por arquivo).
    editors: Vec<EditorWindow>,
    /// Operação do menu esperando confirmação.
    pending_op: Option<PendingOp>,
    /// Operação de arquivo rodando em outra thread.
    op_job: Option<Receiver<(PendingOp, Result<String, String>)>>,
    /// Mensagem da última operação, na barra de status.
    notice: Option<String>,
    /// Pastas abertas/fechadas a manter quando a comparação terminar.
    keep_tree: Option<TreeSnapshot>,
    /// Localizar no diff da tela principal.
    find: FindBar,
    find_matches: Vec<DiffMatch>,
    find_current: Option<usize>,
}

impl RustMergeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Vec<String>) -> Self {
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        let mut app = Self {
            settings,
            comparison: None,
            tree: None,
            scan: None,
            selected: None,
            selected_index: None,
            diff_job: None,
            file_diff: None,
            current_block: None,
            scroll_to_row: None,
            error: None,
            editors: Vec::new(),
            pending_op: None,
            op_job: None,
            notice: None,
            keep_tree: None,
            find: FindBar::default(),
            find_matches: Vec::new(),
            find_current: None,
        };
        if let [left, right, ..] = args.as_slice() {
            app.settings.left = left.clone();
            app.settings.right = right.clone();
            app.start_scan(&cc.egui_ctx);
        }
        app
    }

    fn can_compare(&self) -> bool {
        self.scan.is_none() && !self.settings.left.trim().is_empty() && !self.settings.right.trim().is_empty()
    }

    /// Compara as pastas dos campos de texto.
    fn start_scan(&mut self, ctx: &egui::Context) {
        self.settings.remember_current();
        let left = PathBuf::from(self.settings.left.trim());
        let right = PathBuf::from(self.settings.right.trim());
        self.run_scan(left, right, ctx);
    }

    /// Compara de novo as pastas da comparação atual (depois de copiar, mover, apagar...).
    fn rescan_current(&mut self, ctx: &egui::Context) {
        if let Some(cmp) = &self.comparison {
            let (left, right) = (cmp.left_root.clone(), cmp.right_root.clone());
            self.run_scan(left, right, ctx);
        }
    }

    /// Compara as pastas numa thread separada (que usa várias threads internamente).
    fn run_scan(&mut self, left: PathBuf, right: PathBuf, ctx: &egui::Context) {
        if self.scan.is_some() {
            return;
        }
        // Mesmas pastas: mantém as pastas da árvore abertas/fechadas como estão.
        self.keep_tree = match (&self.comparison, &self.tree) {
            (Some(cmp), Some(tree)) if cmp.left_root == left && cmp.right_root == right => Some(tree.snapshot(cmp)),
            _ => None,
        };
        let options = Options {
            method: self.settings.method,
            threads: self.settings.threads,
            ignore_hidden: self.settings.ignore_hidden,
            extensions: ExtFilter::parse(&self.settings.extensions),
        };
        let progress = Arc::new(Progress::default());
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let job_progress = progress.clone();
        std::thread::spawn(move || {
            let _ = tx.send(compare_folders(&left, &right, options, &job_progress));
            ctx.request_repaint();
        });
        self.scan = Some(ScanJob { rx, progress });
        self.error = None;
    }

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        if let Some(scan) = &self.scan {
            if let Ok(result) = scan.rx.try_recv() {
                self.scan = None;
                match result {
                    Ok(cmp) => {
                        self.tree = Some(match self.keep_tree.take() {
                            Some(snapshot) => TreeState::restore(&cmp, &snapshot),
                            None => TreeState::new(&cmp),
                        });
                        self.selected_index = self.selected.as_deref().and_then(|p| cmp.find(p));
                        self.comparison = Some(cmp);
                        // Recarrega o arquivo aberto, se ele ainda existir.
                        if let Some(index) = self.selected_index {
                            self.open_file(index, ctx);
                        } else {
                            self.file_diff = None;
                        }
                    }
                    Err(e) => self.error = Some(e),
                }
            } else {
                // Atualiza a barra de progresso.
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }

        if let Some(rx) = &self.op_job
            && let Ok((op, result)) = rx.try_recv()
        {
            self.op_job = None;
            self.finish_op(op, result, ctx);
        }

        if let Some(job) = &self.diff_job
            && let Ok(result) = job.rx.try_recv()
        {
            self.diff_job = None;
            match result {
                Ok(diff) => {
                    self.current_block = None;
                    if let Some(first) = diff.blocks.first() {
                        self.current_block = Some(0);
                        self.scroll_to_row = Some(first.start);
                    }
                    self.file_diff = Some(diff);
                    // Com o Localizar aberto, procura no arquivo novo e vai para a primeira ocorrência.
                    let searching = self.find.open && !self.find.query.is_empty();
                    self.update_find_matches(searching);
                }
                Err(e) => self.error = Some(e),
            }
        }
    }

    /// Lê e compara o arquivo numa thread separada, para a janela não travar com arquivos grandes.
    fn open_file(&mut self, index: usize, ctx: &egui::Context) {
        let Some(cmp) = &self.comparison else { return };
        let rel_path = cmp.nodes[index].rel_path.clone();
        let (left_root, right_root) = (cmp.left_root.clone(), cmp.right_root.clone());

        self.selected = Some(rel_path.clone());
        self.selected_index = Some(index);
        self.file_diff = None;
        self.current_block = None;
        self.error = None;

        // Um pedido novo substitui o anterior; o resultado antigo é descartado.
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let job_path = rel_path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(FileDiff::load(&left_root, &right_root, &job_path));
            ctx.request_repaint();
        });
        self.diff_job = Some(DiffJob { rx, rel_path });
    }

    /// Opção do menu do botão direito para os itens selecionados: pergunta antes, se for preciso.
    fn request_action(&mut self, items: Vec<usize>, action: crate::tree_view::FileAction, ctx: &egui::Context) {
        if self.op_job.is_some() || self.scan.is_some() {
            return;
        }
        let Some(cmp) = &self.comparison else { return };
        let op = PendingOp::new(cmp, &items, action);
        if op.is_empty() {
            return;
        }
        if op.needs_confirmation() {
            self.pending_op = Some(op);
        } else {
            self.run_op(op, ctx);
        }
    }

    fn run_op(&mut self, op: PendingOp, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = op.run();
            let _ = tx.send((op, result));
            ctx.request_repaint();
        });
        self.op_job = Some(rx);
        self.notice = None;
    }

    /// Operação terminou: atualiza só o arquivo ou compara tudo de novo.
    fn finish_op(&mut self, op: PendingOp, result: Result<String, String>, ctx: &egui::Context) {
        match result {
            Ok(message) => {
                self.notice = Some(message);
                self.error = None;
                if let Some((old_rel, new_rel)) = op.renamed()
                    && self.selected.as_ref() == Some(&old_rel)
                {
                    self.selected = Some(new_rel);
                }
                if op.needs_rescan() {
                    self.rescan_current(ctx);
                } else if let Some(cmp) = &mut self.comparison {
                    let index = op.items[0].index;
                    cmp.refresh_file(index);
                    if let Some(tree) = &mut self.tree {
                        tree.invalidate();
                    }
                    if self.selected_index == Some(index) {
                        self.open_file(index, ctx);
                    }
                }
            }
            Err(e) => {
                self.error = Some(e);
                // Pode ter feito parte do trabalho: compara de novo para mostrar o estado real.
                self.rescan_current(ctx);
            }
        }
    }

    /// Abre a janela de edição do arquivo (ou traz para a frente, se já estiver aberta).
    fn open_editor(&mut self, index: usize, ctx: &egui::Context) {
        let Some(cmp) = &self.comparison else { return };
        let rel_path = &cmp.nodes[index].rel_path;
        if let Some(editor) = self.editors.iter().find(|e| &e.rel_path == rel_path) {
            ctx.send_viewport_cmd_to(editor.id, egui::ViewportCommand::Focus);
            return;
        }
        match EditorWindow::open(&cmp.left_root, &cmp.right_root, rel_path) {
            Ok(editor) => self.editors.push(editor),
            Err(e) => self.error = Some(e),
        }
    }

    /// Mostra as janelas de edição e, para as que foram fechadas, recompara o arquivo.
    fn show_editors(&mut self, ctx: &egui::Context) {
        for editor in &mut self.editors {
            editor.show(ctx);
        }
        let (closed, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.editors).into_iter().partition(|e| e.closed);
        self.editors = open;

        for editor in closed {
            let Some(cmp) = &mut self.comparison else { continue };
            // Só atualiza se a comparação ainda for das mesmas pastas.
            if cmp.left_root != editor.left_root || cmp.right_root != editor.right_root {
                continue;
            }
            let Some(index) = cmp.find(&editor.rel_path) else { continue };
            cmp.refresh_file(index);
            if let Some(tree) = &mut self.tree {
                tree.invalidate();
            }
            self.open_file(index, ctx);
        }
    }

    fn go_to_block(&mut self, forward: bool) {
        let Some(diff) = &self.file_diff else { return };
        let count = diff.blocks.len();
        if count == 0 {
            return;
        }
        let next = match (self.current_block, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(count - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.current_block = Some(next);
        self.scroll_to_row = Some(diff.blocks[next].start);
    }

    fn top_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("paths").show(ctx, |ui| {
            ui.add_space(4.0);
            let s = &mut self.settings;
            path_row(ui, "Esquerda:", "recent_left", &mut s.left, &s.recent_left, Some("Pastas recentes"));
            path_row(ui, "Direita:", "recent_right", &mut s.right, &s.recent_right, Some("Pastas recentes"));
            let field = path_row(ui, "Extensões:", "recent_extensions", &mut s.extensions, &s.recent_extensions, None)
                .on_hover_text(
                    "Só compara arquivos com estas extensões, separadas por vírgula, ponto e vírgula \
                     ou espaço. Use ! para excluir (ex.: !log). Vazio = todos.\n\
                     Enter compara de novo; senão, vale na próxima comparação (F5).",
                );
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && self.can_compare() {
                self.start_scan(ctx);
            }

            ui.horizontal(|ui| {
                if let Some(scan) = &self.scan {
                    if ui.button("Cancelar").clicked() {
                        scan.progress.cancel.store(true, Ordering::Relaxed);
                    }
                    progress_widget(ui, &scan.progress);
                } else if ui
                    .add_enabled(self.can_compare(), egui::Button::new("Comparar (F5)"))
                    .clicked()
                {
                    self.start_scan(ctx);
                }

                ui.separator();
                ui.label("Método:");
                egui::ComboBox::from_id_salt("method")
                    .selected_text(self.settings.method.description())
                    .show_ui(ui, |ui| {
                        for m in CompareMethod::ALL {
                            ui.selectable_value(&mut self.settings.method, m, m.description());
                        }
                    });

                ui.label("Threads:");
                let auto = folder::default_threads();
                ui.add(
                    egui::DragValue::new(&mut self.settings.threads)
                        .range(0..=folder::cpu_count() * 2)
                        .custom_formatter(move |v, _| if v == 0.0 { format!("auto ({auto})") } else { format!("{v}") }),
                )
                .on_hover_text(
                    "0 = automático (até 8). Mais threads nem sempre é mais rápido: \
                     o disco vira o gargalo. Em HD mecânico, use 1 ou 2.",
                );

                ui.separator();
                ui.checkbox(&mut self.settings.ignore_hidden, "Ignorar arquivos ocultos")
                    .on_hover_text("Vale na próxima comparação (F5).");
                ui.checkbox(&mut self.settings.only_diffs, "Mostrar só diferenças");
            });
            ui.add_space(4.0);
        });
    }

    fn status_bar(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let dark = ui.visuals().dark_mode;
                if let Some(cmp) = &self.comparison {
                    let s = cmp.stats;
                    for (status, count) in [
                        (Status::Equal, s.equal),
                        (Status::Different, s.different),
                        (Status::LeftOnly, s.left_only),
                        (Status::RightOnly, s.right_only),
                        (Status::Error, s.errors),
                    ] {
                        if count > 0 || status != Status::Error {
                            let color = tree_view::status_color(status, dark);
                            ui.label(RichText::new(format!("{}: {count}", status.description())).color(color));
                        }
                    }
                    ui.separator();
                    ui.label(format!("Comparado em {:.2} s", cmp.elapsed.as_secs_f64()));
                }
                if let Some(diff) = &self.file_diff {
                    ui.separator();
                    ui.label(format!("Codificação: {} | {}", show_enc(&diff.left.encoding), show_enc(&diff.right.encoding)));
                }
                if self.op_job.is_some() {
                    ui.separator();
                    ui.spinner();
                    ui.label("Executando...");
                } else if let Some(notice) = &self.notice {
                    ui.separator();
                    ui.label(notice);
                }
            });
        });
    }

    fn tree_panel(&mut self, ctx: &egui::Context) {
        // Id novo ("tree_columns") para a largura padrão maior valer mesmo com a largura antiga salva.
        egui::SidePanel::right("tree_columns")
            .resizable(true)
            .default_width(720.0)
            .min_width(300.0)
            .show(ctx, |ui| {
                ui.heading("Arquivos");
                ui.separator();
                let (Some(cmp), Some(tree)) = (&self.comparison, &mut self.tree) else {
                    ui.weak("Escolha duas pastas e clique em Comparar.");
                    return;
                };
                match tree.show(ui, cmp, self.settings.only_diffs, self.selected_index, &mut self.settings.tree_columns) {
                    Some(TreeClick::Select(index)) => self.open_file(index, ctx),
                    Some(TreeClick::Edit(index)) => {
                        self.open_file(index, ctx);
                        self.open_editor(index, ctx);
                    }
                    Some(TreeClick::Action(items, action)) => self.request_action(items, action, ctx),
                    None => {}
                }
            });
    }

    fn diff_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            if let Some(job) = &self.diff_job {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Carregando {}...", job.rel_path.display()));
                });
                return;
            }
            let Some(diff) = &self.file_diff else {
                ui.centered_and_justified(|ui| {
                    ui.weak("Clique num arquivo da árvore para ver as diferenças.");
                });
                return;
            };

            let mut navigate = None;
            ui.horizontal(|ui| {
                ui.strong(diff.rel_path.display().to_string());
                ui.separator();
                let count = diff.blocks.len();
                if count == 0 {
                    ui.label("Arquivos idênticos");
                } else {
                    if ui.button("⏶ Anterior").on_hover_text("Alt+↑").clicked() {
                        navigate = Some(false);
                    }
                    if ui.button("⏷ Próxima").on_hover_text("Alt+↓").clicked() {
                        navigate = Some(true);
                    }
                    let current = self.current_block.map_or(0, |i| i + 1);
                    ui.label(format!("Diferença {current} de {count}"));
                }
                ui.separator();
                if ui.button("Localizar (Ctrl+F)").clicked() {
                    self.find.show_bar();
                }
            });

            let mut find_event = FindEvent::None;
            if self.find.open {
                let status = find_bar::status(self.find_current, self.find_matches.len());
                find_event = self.find.ui(ui, &status);
            }

            ui.separator();
            if diff.left.is_binary || diff.right.is_binary {
                ui.label("Arquivo binário: a comparação de texto não está disponível.");
            } else {
                let empty: &[DiffMatch] = &[];
                let highlight = Highlight {
                    matches: if self.find.open { &self.find_matches } else { empty },
                    current: self.find_current,
                };
                diff_view::show(ui, diff, self.current_block, &mut self.scroll_to_row, &highlight);
            }

            if let Some(forward) = navigate {
                self.go_to_block(forward);
            }
            self.handle_find_event(find_event);
        });
    }

    /// Recalcula as ocorrências do Localizar no arquivo aberto.
    fn update_find_matches(&mut self, jump_to_first: bool) {
        self.find_matches = match &self.file_diff {
            Some(diff) => find_in_diff(diff, self.find.side, &self.find.query, self.find.case_sensitive),
            None => Vec::new(),
        };
        self.find_current = if self.find_matches.is_empty() { None } else { Some(0) };
        if jump_to_first && let Some(first) = self.find_matches.first() {
            self.scroll_to_row = Some(first.row);
        }
    }

    fn handle_find_event(&mut self, event: FindEvent) {
        match event {
            FindEvent::Changed => self.update_find_matches(true),
            FindEvent::Next | FindEvent::Prev => {
                self.find_current = find_bar::step(self.find_current, self.find_matches.len(), event == FindEvent::Next);
                if let Some(m) = self.find_current.and_then(|i| self.find_matches.get(i)) {
                    self.scroll_to_row = Some(m.row);
                }
            }
            FindEvent::None | FindEvent::Closed => {}
        }
    }
}

/// Linha com rótulo, campo de texto e menu com os valores usados antes.
/// Com `folder_hint` (pastas), mostra também o botão de escolher pasta; sem ele (filtro de
/// extensões), deixa o espaço do botão vazio para os campos ficarem alinhados.
/// Devolve a resposta do campo de texto.
fn path_row(
    ui: &mut egui::Ui,
    label: &str,
    menu_id: &str,
    input: &mut String,
    recent: &[String],
    folder_hint: Option<&str>,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized([70.0, 20.0], egui::Label::new(label));
        let button = 28.0;
        let width = ui.available_width() - 2.0 * (button + ui.spacing().item_spacing.x);
        let mut edit = egui::TextEdit::singleline(input).desired_width(width);
        if folder_hint.is_none() {
            edit = edit.hint_text("todas (ex.: rs, toml, !log)");
        }
        let field = ui.add(edit);

        ui.push_id(menu_id, |ui| {
            ui.add_enabled_ui(!recent.is_empty(), |ui| {
                ui.menu_button("⏷", |ui| {
                    for value in recent {
                        if ui.button(value).clicked() {
                            *input = value.clone();
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text(folder_hint.unwrap_or("Filtros usados antes"));
            });
        });

        if folder_hint.is_some() {
            if ui
                .add_sized([button, 20.0], egui::Button::new("..."))
                .on_hover_text("Escolher pasta")
                .clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                *input = dir.display().to_string();
            }
        } else {
            ui.add_space(button);
        }
        field
    })
    .inner
}

fn progress_widget(ui: &mut egui::Ui, progress: &Progress) {
    ui.spinner();
    if progress.comparing.load(Ordering::Relaxed) {
        let total = progress.files_to_compare.load(Ordering::Relaxed);
        let done = progress.files_compared.load(Ordering::Relaxed);
        let mb = progress.bytes_compared.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0);
        let fraction = if total == 0 { 1.0 } else { done as f32 / total as f32 };
        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_width(320.0)
                .text(format!("{done} de {total} arquivos · {mb:.0} MB")),
        );
    } else {
        let dirs = progress.dirs_scanned.load(Ordering::Relaxed);
        ui.label(format!("Lendo pastas... {dirs}"));
    }
}

fn show_enc(enc: &str) -> &str {
    if enc.is_empty() { "—" } else { enc }
}

impl eframe::App for RustMergeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_jobs(ctx);

        // Ctrl+F, F3, Shift+F3 e Esc do Localizar (só valem com um arquivo aberto).
        if self.file_diff.is_some() {
            let event = self.find.handle_shortcuts(ctx);
            self.handle_find_event(event);
        }

        let (next, prev, refresh) = ctx.input(|i| {
            (
                i.modifiers.alt && i.key_pressed(Key::ArrowDown),
                i.modifiers.alt && i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::F5),
            )
        });
        if next {
            self.go_to_block(true);
        }
        if prev {
            self.go_to_block(false);
        }
        if refresh && self.can_compare() {
            self.start_scan(ctx);
        }

        // Não deixa fechar a janela principal com edições não salvas: pergunta em cada editor.
        if ctx.input(|i| i.viewport().close_requested()) && self.editors.iter().any(EditorWindow::is_dirty) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            for editor in self.editors.iter_mut().filter(|e| e.is_dirty()) {
                editor.request_close();
                ctx.send_viewport_cmd_to(editor.id, egui::ViewportCommand::Focus);
            }
        }

        // A ordem importa: painéis laterais antes do central.
        self.top_panel(ctx);
        self.status_bar(ctx);
        self.tree_panel(ctx);
        self.diff_panel(ctx);
        self.show_editors(ctx);

        if let Some(op) = &mut self.pending_op {
            match op.show_modal(ctx) {
                ModalResult::Waiting => {}
                ModalResult::Cancel => self.pending_op = None,
                ModalResult::Confirm => {
                    if let Some(op) = self.pending_op.take() {
                        self.run_op(op, ctx);
                    }
                }
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}
