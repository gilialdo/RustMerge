//! Menu do botão direito: confirmação (janela modal) e execução das operações de arquivo,
//! para um ou vários itens selecionados.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Key, RichText};

use rustmerge::folder::Comparison;
use rustmerge::ops::{Side, copy_item, delete_item, move_item, rename_item, validate_name};

use crate::tree_view::FileAction;

pub enum ModalResult {
    Waiting,
    Confirm,
    Cancel,
}

/// Quantos caminhos listar na confirmação antes de resumir ("... e mais N").
const MAX_LISTED: usize = 10;

/// Um item da operação.
pub struct OpItem {
    pub index: usize,
    pub rel_path: PathBuf,
    is_dir: bool,
    left: PathBuf,
    right: PathBuf,
    in_left: bool,
    in_right: bool,
}

impl OpItem {
    fn path(&self, side: Side) -> &Path {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn exists(&self, side: Side) -> bool {
        match side {
            Side::Left => self.in_left,
            Side::Right => self.in_right,
        }
    }

    fn kind(&self) -> &'static str {
        if self.is_dir { "a pasta" } else { "o arquivo" }
    }

    /// Lados que serão apagados neste item.
    fn delete_sides(&self, action: FileAction) -> Vec<Side> {
        match action {
            FileAction::Delete(Some(side)) => vec![side],
            FileAction::Delete(None) => vec![Side::Left, Side::Right],
            _ => Vec::new(),
        }
        .into_iter()
        .filter(|s| self.exists(*s))
        .collect()
    }

    /// O item participa da operação? (Ex.: "copiar para a direita" só vale para quem existe na esquerda.)
    fn applies(&self, action: FileAction) -> bool {
        match action {
            FileAction::Copy(to) | FileAction::Move(to) => self.exists(to.other()),
            FileAction::Delete(_) => !self.delete_sides(action).is_empty(),
            FileAction::Rename => true,
        }
    }

    /// Executa a operação neste item.
    fn run(&self, action: FileAction, new_name: &str) -> Result<(), String> {
        let rel = self.rel_path.display();
        match action {
            FileAction::Copy(to) => {
                copy_item(self.path(to.other()), self.path(to)).map_err(|e| format!("Erro ao copiar {rel}: {e}"))
            }
            FileAction::Move(to) => {
                move_item(self.path(to.other()), self.path(to)).map_err(|e| format!("Erro ao mover {rel}: {e}"))
            }
            FileAction::Delete(_) => {
                for side in self.delete_sides(action) {
                    delete_item(self.path(side))?;
                }
                Ok(())
            }
            FileAction::Rename => {
                let sides: Vec<Side> = [Side::Left, Side::Right].into_iter().filter(|s| self.exists(*s)).collect();
                // Confere os dois lados antes, para não renomear só um deles.
                for side in &sides {
                    let target = self.path(*side).with_file_name(new_name);
                    let same = target.to_string_lossy().to_lowercase() == self.path(*side).to_string_lossy().to_lowercase();
                    if target.exists() && !same {
                        return Err(format!("Já existe {}.", target.display()));
                    }
                }
                for side in sides {
                    rename_item(self.path(side), new_name)?;
                }
                Ok(())
            }
        }
    }
}

/// Uma operação escolhida no menu, com tudo o que é preciso para executá-la em outra thread.
pub struct PendingOp {
    pub action: FileAction,
    /// Só os itens aos quais a operação se aplica.
    pub items: Vec<OpItem>,
    new_name: String,
    error: Option<String>,
    focus_requested: bool,
}

impl PendingOp {
    pub fn new(cmp: &Comparison, indices: &[usize], action: FileAction) -> Self {
        let items: Vec<OpItem> = indices
            .iter()
            .map(|&index| {
                let node = &cmp.nodes[index];
                OpItem {
                    index,
                    rel_path: node.rel_path.clone(),
                    is_dir: node.is_dir,
                    left: cmp.left_root.join(&node.rel_path),
                    right: cmp.right_root.join(&node.rel_path),
                    in_left: node.in_left,
                    in_right: node.in_right,
                }
            })
            .filter(|item| item.applies(action))
            .collect();
        let new_name = indices.first().map(|&i| cmp.nodes[i].name.clone()).unwrap_or_default();
        Self { action, items, new_name, error: None, focus_requested: false }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn single(&self) -> Option<&OpItem> {
        (self.items.len() == 1).then(|| &self.items[0])
    }

    /// Ex.: "o arquivo" ou "3 itens".
    fn what(&self) -> String {
        match self.single() {
            Some(item) => item.kind().to_string(),
            None => format!("{} itens", self.items.len()),
        }
    }

    /// Copiar sem substituir nada não precisa de confirmação; o resto pergunta antes.
    pub fn needs_confirmation(&self) -> bool {
        match self.action {
            FileAction::Copy(to) => self.items.iter().any(|i| i.path(to).exists()),
            _ => true,
        }
    }

    /// Basta recomparar o arquivo (um só arquivo, que continua existindo em algum lado)?
    /// Senão, é preciso comparar tudo de novo.
    pub fn needs_rescan(&self) -> bool {
        let Some(item) = self.single() else { return true };
        match self.action {
            _ if item.is_dir => true,
            FileAction::Rename => true,
            FileAction::Delete(_) => {
                let deleted = item.delete_sides(self.action);
                [Side::Left, Side::Right].iter().all(|s| !item.exists(*s) || deleted.contains(s))
            }
            _ => false,
        }
    }

    /// Caminho relativo depois de renomear (operação de um item só).
    pub fn renamed(&self) -> Option<(PathBuf, PathBuf)> {
        let item = self.single().filter(|_| self.action == FileAction::Rename)?;
        Some((item.rel_path.clone(), item.rel_path.with_file_name(self.new_name.trim())))
    }

    /// Lista de caminhos para a confirmação, resumida se for longa.
    fn list_paths(&self, ui: &mut egui::Ui, paths: Vec<PathBuf>) {
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for path in paths.iter().take(MAX_LISTED) {
                ui.monospace(path.display().to_string());
            }
            if paths.len() > MAX_LISTED {
                ui.weak(format!("... e mais {}", paths.len() - MAX_LISTED));
            }
        });
    }

    /// Janela de confirmação. Chamar a cada quadro enquanto a operação estiver pendente.
    pub fn show_modal(&mut self, ctx: &egui::Context) -> ModalResult {
        let mut result = ModalResult::Waiting;
        let response = egui::Modal::new(egui::Id::new("file_op_modal")).show(ctx, |ui| {
            ui.set_width(560.0);
            let (title, button) = match self.action {
                FileAction::Copy(to) => (format!("Copiar para a {}", to.name()), "Substituir"),
                FileAction::Move(to) => (format!("Mover para a {}", to.name()), "Mover"),
                FileAction::Delete(_) => ("Apagar".to_string(), "Mandar para a Lixeira"),
                FileAction::Rename => ("Renomear".to_string(), "Renomear"),
            };
            ui.heading(title);
            ui.add_space(6.0);

            match self.action {
                FileAction::Copy(to) => {
                    let existing: Vec<PathBuf> =
                        self.items.iter().filter(|i| i.path(to).exists()).map(|i| i.path(to).to_path_buf()).collect();
                    ui.label(format!(
                        "Copiar {} da {} para a {}. Já existe{} na {} e será substituído:",
                        self.what(),
                        to.other().name(),
                        to.name(),
                        if existing.len() > 1 { "m" } else { "" },
                        to.name()
                    ));
                    self.list_paths(ui, existing);
                    if self.items.iter().any(|i| i.is_dir) {
                        ui.weak("Pastas são mescladas: arquivos com o mesmo nome são substituídos.");
                    }
                }
                FileAction::Move(to) => {
                    ui.label(format!("Mover {} da {} para a {}?", self.what(), to.other().name(), to.name()));
                    self.list_paths(ui, self.items.iter().map(|i| i.rel_path.clone()).collect());
                    if self.items.iter().any(|i| i.path(to).exists()) {
                        ui.colored_label(ui.visuals().warn_fg_color, "Destinos que já existem serão substituídos.");
                    }
                }
                FileAction::Delete(_) => {
                    ui.label(format!("Mandar para a Lixeira {}:", self.what()));
                    let paths = self
                        .items
                        .iter()
                        .flat_map(|i| i.delete_sides(self.action).into_iter().map(|s| i.path(s).to_path_buf()))
                        .collect();
                    self.list_paths(ui, paths);
                }
                FileAction::Rename => {
                    if let Some(item) = self.single() {
                        let sides: Vec<&str> =
                            [Side::Left, Side::Right].into_iter().filter(|s| item.exists(*s)).map(Side::name).collect();
                        ui.label(format!("Novo nome (vale para: {}):", sides.join(" e ")));
                    }
                    let edit = ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(f32::INFINITY));
                    if !self.focus_requested {
                        edit.request_focus();
                        self.focus_requested = true;
                    }
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        result = self.confirm();
                    }
                }
            }
            if let Some(err) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new(button).strong()).clicked() {
                    result = self.confirm();
                }
                if ui.button("Cancelar").clicked() {
                    result = ModalResult::Cancel;
                }
            });
        });
        if response.should_close() && matches!(result, ModalResult::Waiting) {
            result = ModalResult::Cancel;
        }
        result
    }

    fn confirm(&mut self) -> ModalResult {
        if self.action == FileAction::Rename {
            if let Err(e) = validate_name(&self.new_name) {
                self.error = Some(e);
                return ModalResult::Waiting;
            }
            if self.single().and_then(|i| i.rel_path.file_name()).is_some_and(|n| n == self.new_name.trim()) {
                return ModalResult::Cancel;
            }
        }
        ModalResult::Confirm
    }

    /// Executa em todos os itens (roda em outra thread). Continua mesmo se algum falhar
    /// e devolve a mensagem para a barra de status.
    pub fn run(&self) -> Result<String, String> {
        let name = self.new_name.trim();
        let errors: Vec<String> = self.items.iter().filter_map(|item| item.run(self.action, name).err()).collect();
        let done = self.items.len() - errors.len();

        if !errors.is_empty() {
            let mut message = if self.items.len() == 1 {
                errors[0].clone()
            } else {
                format!("{} de {} itens falharam. {}", errors.len(), self.items.len(), errors[0])
            };
            if errors.len() > 1 {
                message.push_str(&format!(" (e mais {} erro(s))", errors.len() - 1));
            }
            return Err(message);
        }

        let what = match self.single() {
            Some(item) => item.rel_path.display().to_string(),
            None => format!("{done} itens"),
        };
        Ok(match self.action {
            FileAction::Copy(to) => format!("Copiado para a {}: {what}", to.name()),
            FileAction::Move(to) => format!("Movido para a {}: {what}", to.name()),
            FileAction::Delete(_) => format!("Enviado para a Lixeira: {what}"),
            FileAction::Rename => format!("Renomeado: {what} para {name}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustmerge::folder::{Options, Progress, compare_folders};
    use std::fs;

    #[test]
    fn copia_varios_itens() {
        let root = std::env::temp_dir().join(format!("rustmerge_actions_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (l, r) = (root.join("l"), root.join("r"));
        fs::create_dir_all(l.join("pasta")).unwrap();
        fs::create_dir_all(&r).unwrap();
        fs::write(l.join("a.txt"), "a").unwrap();
        fs::write(l.join("pasta/b.txt"), "b").unwrap();
        fs::write(r.join("so_direita.txt"), "x").unwrap();
        let cmp = compare_folders(&l, &r, Options::default(), &Progress::default()).unwrap();
        let all: Vec<usize> = cmp.roots.clone();

        // "Copiar para a direita" ignora o item que só existe na direita.
        let op = PendingOp::new(&cmp, &all, FileAction::Copy(Side::Right));
        assert_eq!(op.items.len(), 2);
        assert!(!op.needs_confirmation());
        assert!(op.needs_rescan());
        op.run().unwrap();
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "a");
        assert_eq!(fs::read_to_string(r.join("pasta/b.txt")).unwrap(), "b");

        // Copiar de novo: agora os destinos existem e pede confirmação.
        let cmp = compare_folders(&l, &r, Options::default(), &Progress::default()).unwrap();
        let op = PendingOp::new(&cmp, &cmp.roots.clone(), FileAction::Copy(Side::Right));
        assert!(op.needs_confirmation());

        fs::remove_dir_all(&root).unwrap();
    }
}
