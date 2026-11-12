//! Diff de linhas alinhado lado a lado e diff de caracteres dentro da linha.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use similar::{Algorithm, ChangeTag, DiffTag, TextDiff};

use crate::text::TextFile;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Equal,
    Changed,
    LeftOnly,
    RightOnly,
}

/// Uma linha da tela: índice da linha em cada lado (None = linha vazia de preenchimento).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub kind: RowKind,
}

pub struct FileDiff {
    pub rel_path: PathBuf,
    pub left: TextFile,
    pub right: TextFile,
    pub rows: Vec<Row>,
    /// Intervalos de linhas da tela que formam cada bloco de diferença.
    pub blocks: Vec<Range<usize>>,
}

impl FileDiff {
    pub fn new(rel_path: PathBuf, left: TextFile, right: TextFile) -> Self {
        let (rows, blocks) = align(&left.lines, &right.lines);
        Self { rel_path, left, right, rows, blocks }
    }

    /// Lê o arquivo `rel_path` nas duas raízes (o lado onde ele não existe fica vazio) e calcula o diff.
    pub fn load(left_root: &Path, right_root: &Path, rel_path: &Path) -> Result<Self, String> {
        let existing = |root: &Path| Some(root.join(rel_path)).filter(|p| p.is_file());
        let read = |root: &Path| {
            crate::text::load(existing(root).as_deref())
                .map_err(|e| format!("Erro ao ler {}: {e}", root.join(rel_path).display()))
        };
        Ok(Self::new(rel_path.to_path_buf(), read(left_root)?, read(right_root)?))
    }
}

pub fn align(left: &[String], right: &[String]) -> (Vec<Row>, Vec<Range<usize>>) {
    let l: Vec<&str> = left.iter().map(String::as_str).collect();
    let r: Vec<&str> = right.iter().map(String::as_str).collect();
    // O tempo limite evita travar em arquivos enormes muito diferentes (o resultado fica menos preciso).
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Patience)
        .timeout(Duration::from_secs(5))
        .diff_slices(&l, &r);

    let mut rows = Vec::with_capacity(l.len().max(r.len()));
    let mut blocks: Vec<Range<usize>> = Vec::new();

    for op in diff.ops() {
        let (tag, lr, rr) = op.as_tag_tuple();
        let start = rows.len();
        match tag {
            DiffTag::Equal => rows.extend(lr.zip(rr).map(|(a, b)| Row {
                left: Some(a),
                right: Some(b),
                kind: RowKind::Equal,
            })),
            DiffTag::Delete => rows.extend(lr.map(|a| Row { left: Some(a), right: None, kind: RowKind::LeftOnly })),
            DiffTag::Insert => rows.extend(rr.map(|b| Row { left: None, right: Some(b), kind: RowKind::RightOnly })),
            DiffTag::Replace => {
                for i in 0..lr.len().max(rr.len()) {
                    let left = (i < lr.len()).then(|| lr.start + i);
                    let right = (i < rr.len()).then(|| rr.start + i);
                    let kind = match (left, right) {
                        (Some(_), Some(_)) => RowKind::Changed,
                        (Some(_), None) => RowKind::LeftOnly,
                        _ => RowKind::RightOnly,
                    };
                    rows.push(Row { left, right, kind });
                }
            }
        }
        if tag != DiffTag::Equal {
            let end = rows.len();
            // Remoção seguida de inserção vira um bloco só.
            match blocks.last_mut() {
                Some(last) if last.end == start => last.end = end,
                _ => blocks.push(start..end),
            }
        }
    }
    (rows, blocks)
}

/// Tipo de cada linha de cada lado (usado para colorir o editor).
pub fn line_marks(left: &[String], right: &[String]) -> (Vec<RowKind>, Vec<RowKind>) {
    let (rows, _) = align(left, right);
    let mut l = vec![RowKind::Equal; left.len()];
    let mut r = vec![RowKind::Equal; right.len()];
    for row in rows {
        if let Some(i) = row.left {
            l[i] = row.kind;
        }
        if let Some(j) = row.right {
            r[j] = row.kind;
        }
    }
    (l, r)
}

/// Trecho de texto e se ele mudou em relação ao outro lado.
pub type Segment = (String, bool);

/// Diferença de caracteres entre duas linhas alteradas.
pub fn inline_segments(old: &str, new: &str) -> (Vec<Segment>, Vec<Segment>) {
    // Linhas muito longas: marca a linha inteira para não travar a tela.
    if old.len() + new.len() > 4000 {
        return (vec![(old.to_string(), true)], vec![(new.to_string(), true)]);
    }
    let diff = TextDiff::from_chars(old, new);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for change in diff.iter_all_changes() {
        let value = change.value();
        match change.tag() {
            ChangeTag::Equal => {
                push(&mut l, value, false);
                push(&mut r, value, false);
            }
            ChangeTag::Delete => push(&mut l, value, true),
            ChangeTag::Insert => push(&mut r, value, true),
        }
    }
    (l, r)
}

fn push(segments: &mut Vec<Segment>, text: &str, changed: bool) {
    match segments.last_mut() {
        Some((s, c)) if *c == changed => s.push_str(text),
        _ => segments.push((text.to_string(), changed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn alinha_linhas() {
        let (rows, blocks) = align(&lines("a\nb\nc\nd"), &lines("a\nB\nc\nd\ne"));
        let kinds: Vec<RowKind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![RowKind::Equal, RowKind::Changed, RowKind::Equal, RowKind::Equal, RowKind::RightOnly]
        );
        assert_eq!(blocks, vec![1..2, 4..5]);
        assert_eq!(rows[4], Row { left: None, right: Some(4), kind: RowKind::RightOnly });
    }

    #[test]
    fn arquivo_so_de_um_lado() {
        let (rows, blocks) = align(&lines("x\ny"), &[]);
        assert!(rows.iter().all(|r| r.kind == RowKind::LeftOnly));
        assert_eq!(blocks, vec![0..2]);
    }

    #[test]
    fn marca_linhas_de_cada_lado() {
        let (l, r) = line_marks(&lines("a\nb\nc"), &lines("a\nB\nc\nd"));
        assert_eq!(l, vec![RowKind::Equal, RowKind::Changed, RowKind::Equal]);
        assert_eq!(r, vec![RowKind::Equal, RowKind::Changed, RowKind::Equal, RowKind::RightOnly]);
    }

    #[test]
    fn diferenca_dentro_da_linha() {
        let (l, r) = inline_segments("let x = 1;", "let x = 2;");
        assert_eq!(l, vec![("let x = ".into(), false), ("1".into(), true), (";".into(), false)]);
        assert_eq!(r, vec![("let x = ".into(), false), ("2".into(), true), (";".into(), false)]);
    }
}
