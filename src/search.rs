//! Busca de texto (Localizar), com ou sem diferenciar maiúsculas de minúsculas.

use std::ops::Range;

/// Todas as ocorrências de `needle` em `haystack`, como intervalos de bytes (sem sobreposição).
pub fn find_all(haystack: &str, needle: &str, case_sensitive: bool) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    if case_sensitive {
        return haystack.match_indices(needle).map(|(i, m)| i..i + m.len()).collect();
    }

    // Sem diferenciar maiúsculas: compara caractere a caractere, mantendo as posições originais.
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    let mut result = Vec::new();
    let mut start = 0;
    while start < haystack.len() {
        if let Some(len) = match_at(&haystack[start..], &needle) {
            result.push(start..start + len);
            start += len.max(1);
        } else {
            start += haystack[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    result
}

/// Se o texto começa com `needle` (já em minúsculas), devolve quantos bytes a ocorrência ocupa.
fn match_at(text: &str, needle: &[char]) -> Option<usize> {
    let mut pending = needle.iter();
    let mut next = pending.next();
    for (i, c) in text.char_indices() {
        for lower in c.to_lowercase() {
            match next {
                Some(&n) if n == lower => next = pending.next(),
                _ => return None,
            }
        }
        if next.is_none() {
            return Some(i + c.len_utf8());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localiza_texto() {
        assert_eq!(find_all("abc abc", "bc", true), vec![1..3, 5..7]);
        assert_eq!(find_all("Olá OLÁ olá", "olá", false), vec![0..4, 5..9, 10..14]);
        assert_eq!(find_all("Olá OLÁ olá", "olá", true), vec![10..14]);
        assert_eq!(find_all("aaaa", "aa", false), vec![0..2, 2..4]);
        assert!(find_all("abc", "", false).is_empty());
        assert!(find_all("abc", "x", false).is_empty());
    }
}
