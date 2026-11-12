//! Formatação de tamanho e data para exibir na árvore.

use std::time::SystemTime;

use chrono::{DateTime, Local};

/// Tamanho legível, com vírgula decimal: "512 B", "12,4 KB", "3,0 MB".
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
}

/// Data e hora locais: "11/11/2026 21:03".
pub fn format_date(time: SystemTime) -> String {
    DateTime::<Local>::from(time).format("%d/%m/%Y %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formata_tamanho() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1,0 KB");
        assert_eq!(format_size(12_700), "12,4 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3,0 MB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5,0 GB");
    }

    #[test]
    fn formata_data() {
        let text = format_date(SystemTime::now());
        // dd/mm/aaaa hh:mm
        assert_eq!(text.len(), 16);
        assert_eq!(&text[2..3], "/");
        assert_eq!(&text[13..14], ":");
    }
}
