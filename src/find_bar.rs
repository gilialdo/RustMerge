//! Barra "Localizar" usada na tela principal e na janela de edição.
//! Atalhos: Ctrl+F abre, Enter/F3 vai para a próxima, Shift+Enter/Shift+F3 para a anterior, Esc fecha.

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

use rustmerge::ops::Side;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FindEvent {
    None,
    /// O texto procurado ou a opção de maiúsculas mudou.
    Changed,
    Next,
    Prev,
    Closed,
}

pub struct FindBar {
    pub open: bool,
    pub query: String,
    pub case_sensitive: bool,
    /// Lado onde a busca é feita (um de cada vez).
    pub side: Side,
    focus: bool,
}

impl Default for FindBar {
    fn default() -> Self {
        Self { open: false, query: String::new(), case_sensitive: false, side: Side::Left, focus: false }
    }
}

impl FindBar {
    pub fn show_bar(&mut self) {
        self.open = true;
        self.focus = true;
    }

    /// Atalhos de teclado. Chamar uma vez por quadro, antes de desenhar a barra.
    pub fn handle_shortcuts(&mut self, ctx: &egui::Context) -> FindEvent {
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::F))) {
            self.show_bar();
        }
        if !self.open {
            return FindEvent::None;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::SHIFT, Key::F3))) {
            return FindEvent::Prev;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F3)) {
            return FindEvent::Next;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.open = false;
            return FindEvent::Closed;
        }
        FindEvent::None
    }

    /// Desenha a barra. `status` é o texto do contador, por exemplo "2 de 5".
    pub fn ui(&mut self, ui: &mut egui::Ui, status: &str) -> FindEvent {
        let mut event = FindEvent::None;
        ui.horizontal(|ui| {
            ui.label("Localizar na");
            for (side, label) in [(Side::Left, "Esquerda"), (Side::Right, "Direita")] {
                if ui.selectable_value(&mut self.side, side, label).changed() {
                    event = FindEvent::Changed;
                }
            }
            ui.label(":");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(ui.id().with("find_query"))
                    .hint_text("texto a localizar")
                    .desired_width(260.0),
            );
            if self.focus {
                edit.request_focus();
                self.focus = false;
            }
            if edit.changed() {
                event = FindEvent::Changed;
            }
            // Enter tira o foco do campo; devolve o foco para dar Enter várias vezes.
            if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                event = if ui.input(|i| i.modifiers.shift) { FindEvent::Prev } else { FindEvent::Next };
                edit.request_focus();
            }
            if ui.button("⏶").on_hover_text("Anterior (Shift+F3)").clicked() {
                event = FindEvent::Prev;
            }
            if ui.button("⏷").on_hover_text("Próxima (F3)").clicked() {
                event = FindEvent::Next;
            }
            if ui.checkbox(&mut self.case_sensitive, "Diferenciar maiúsculas").changed() {
                event = FindEvent::Changed;
            }
            if !self.query.is_empty() {
                ui.label(status);
            }
            if ui.button("Fechar").on_hover_text("Esc").clicked() {
                self.open = false;
                event = FindEvent::Closed;
            }
        });
        event
    }
}

/// Próximo índice ao navegar (dá a volta no fim e no começo).
pub fn step(current: Option<usize>, count: usize, forward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => count - 1,
        (Some(i), true) => (i + 1) % count,
        (Some(i), false) => (i + count - 1) % count,
    })
}

/// Texto do contador.
pub fn status(current: Option<usize>, count: usize) -> String {
    match (current, count) {
        (_, 0) => "Nenhuma ocorrência".to_string(),
        (Some(i), n) => format!("{} de {n}", i + 1),
        (None, n) => format!("{n} ocorrências"),
    }
}
