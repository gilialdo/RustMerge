//! Preferências salvas entre execuções (pastas recentes, método, threads).
//! O eframe grava em %APPDATA%\RustMerge\data\app.ron.

use rustmerge::folder::CompareMethod;
use serde::{Deserialize, Serialize};

pub const MAX_RECENT: usize = 15;

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub left: String,
    pub right: String,
    pub recent_left: Vec<String>,
    pub recent_right: Vec<String>,
    pub method: CompareMethod,
    /// 0 = automático.
    pub threads: usize,
    pub only_diffs: bool,
    pub ignore_hidden: bool,
    /// Filtro de extensões (ex.: "rs, toml, !log"); vazio = todos os arquivos.
    pub extensions: String,
    pub recent_extensions: Vec<String>,
    /// Larguras das colunas da árvore (Situação, tamanhos e datas).
    pub tree_columns: [f32; crate::tree_view::COLUMN_COUNT],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            left: String::new(),
            right: String::new(),
            recent_left: Vec::new(),
            recent_right: Vec::new(),
            method: CompareMethod::Content,
            threads: 0,
            only_diffs: false,
            // Marcado por padrão (também vale para configurações salvas antes desta opção existir).
            ignore_hidden: true,
            extensions: String::new(),
            recent_extensions: Vec::new(),
            tree_columns: crate::tree_view::DEFAULT_COLUMNS,
        }
    }
}

impl Settings {
    /// Guarda as pastas e o filtro atuais no topo das listas de recentes.
    pub fn remember_current(&mut self) {
        remember(&mut self.recent_left, &self.left);
        remember(&mut self.recent_right, &self.right);
        remember(&mut self.recent_extensions, &self.extensions);
    }
}

pub fn remember(list: &mut Vec<String>, path: &str) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    list.retain(|p| !same_path(p, path));
    list.insert(0, path.to_string());
    list.truncate(MAX_RECENT);
}

fn same_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let s = s.trim().trim_end_matches(['\\', '/']);
        if cfg!(windows) { s.to_lowercase().replace('/', "\\") } else { s.to_string() }
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarda_recentes_sem_repetir() {
        let mut list = Vec::new();
        remember(&mut list, r"C:\a");
        remember(&mut list, r"C:\b");
        remember(&mut list, r"C:\A\");
        assert_eq!(list[0].to_lowercase(), r"c:\a\".to_lowercase());
        assert_eq!(list.len(), 2);
        for i in 0..30 {
            remember(&mut list, &format!(r"C:\p{i}"));
        }
        assert_eq!(list.len(), MAX_RECENT);
    }
}
