//! Leitura e gravação de arquivos de texto, preservando codificação e quebra de linha.

use std::io;
use std::path::Path;

use encoding_rs::{UTF_16BE, UTF_16LE, WINDOWS_1252};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Windows1252,
}

impl TextEncoding {
    pub fn name(self) -> &'static str {
        match self {
            TextEncoding::Utf8 => "UTF-8",
            TextEncoding::Utf8Bom => "UTF-8 (BOM)",
            TextEncoding::Utf16Le => "UTF-16LE (BOM)",
            TextEncoding::Utf16Be => "UTF-16BE (BOM)",
            TextEncoding::Windows1252 => "windows-1252",
        }
    }
}

/// Texto decodificado, com as quebras de linha normalizadas para `\n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub encoding: TextEncoding,
    /// O arquivo original usava `\r\n`.
    pub crlf: bool,
}

/// Decodifica bytes; `None` se o conteúdo parecer binário.
pub fn decode_text(bytes: &[u8]) -> Option<Decoded> {
    let (raw, encoding) = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (String::from_utf8_lossy(rest).into_owned(), TextEncoding::Utf8Bom)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (UTF_16LE.decode_without_bom_handling(rest).0.into_owned(), TextEncoding::Utf16Le)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (UTF_16BE.decode_without_bom_handling(rest).0.into_owned(), TextEncoding::Utf16Be)
    } else if bytes[..bytes.len().min(8000)].contains(&0) {
        return None;
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        (s.to_string(), TextEncoding::Utf8)
    } else {
        (WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(), TextEncoding::Windows1252)
    };
    let crlf = raw.contains("\r\n");
    let text = if crlf { raw.replace("\r\n", "\n") } else { raw };
    Some(Decoded { text, encoding, crlf })
}

/// Codifica de volta para bytes, na mesma codificação e quebra de linha do original.
pub fn encode_text(text: &str, encoding: TextEncoding, crlf: bool) -> Result<Vec<u8>, String> {
    let text = if crlf { text.replace('\n', "\r\n") } else { text.to_string() };
    Ok(match encoding {
        TextEncoding::Utf8 => text.into_bytes(),
        TextEncoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        TextEncoding::Utf16Le => [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect(),
        TextEncoding::Utf16Be => [0xFE, 0xFF].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect(),
        TextEncoding::Windows1252 => {
            let (bytes, _, had_errors) = WINDOWS_1252.encode(&text);
            if had_errors {
                return Err("O texto tem caracteres que não existem em windows-1252.".into());
            }
            bytes.into_owned()
        }
    })
}

/// Lê um arquivo de texto; `Ok(None)` se for binário.
pub fn read_text(path: &Path) -> io::Result<Option<Decoded>> {
    Ok(decode_text(&std::fs::read(path)?))
}

/// Grava o texto, criando as pastas que faltarem.
pub fn write_text(path: &Path, text: &str, encoding: TextEncoding, crlf: bool) -> Result<(), String> {
    let bytes = encode_text(text, encoding, crlf)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Erro ao criar {}: {e}", parent.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("Erro ao salvar {}: {e}", path.display()))
}

/// Arquivo pronto para o diff: só as linhas.
#[derive(Debug, Default, Clone)]
pub struct TextFile {
    pub lines: Vec<String>,
    /// Nome da codificação detectada; vazio quando o arquivo não existe deste lado.
    pub encoding: String,
    pub is_binary: bool,
}

pub fn load(path: Option<&Path>) -> io::Result<TextFile> {
    match path {
        Some(p) => Ok(decode(&std::fs::read(p)?)),
        None => Ok(TextFile::default()),
    }
}

pub fn decode(bytes: &[u8]) -> TextFile {
    match decode_text(bytes) {
        Some(d) => TextFile {
            lines: d.text.lines().map(str::to_string).collect(),
            encoding: d.encoding.name().to_string(),
            is_binary: false,
        },
        None => TextFile { lines: Vec::new(), encoding: "binário".into(), is_binary: true },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecta_codificacoes() {
        let utf8 = decode("olá\r\nmundo\n".as_bytes());
        assert_eq!(utf8.lines, vec!["olá", "mundo"]);
        assert_eq!(utf8.encoding, "UTF-8");

        let latin1 = decode(&[b'o', b'l', 0xE1]);
        assert_eq!(latin1.lines, vec!["olá"]);
        assert_eq!(latin1.encoding, "windows-1252");

        let utf16 = decode(&[0xFF, 0xFE, b'a', 0, b'\n', 0, b'b', 0]);
        assert_eq!(utf16.lines, vec!["a", "b"]);

        assert!(decode(&[1, 2, 0, 3]).is_binary);
    }

    #[test]
    fn grava_na_mesma_codificacao() {
        let samples: Vec<Vec<u8>> = vec![
            "olá\r\nmundo\r\n".as_bytes().to_vec(),
            "olá\nmundo".as_bytes().to_vec(),
            [&[0xEF, 0xBB, 0xBF][..], "olá\n".as_bytes()].concat(),
            vec![0xFF, 0xFE, b'a', 0, 0xE1, 0, b'\r', 0, b'\n', 0],
            vec![0xFE, 0xFF, 0, b'a', 0, b'\n'],
            vec![b'o', b'l', 0xE1, b'\r', b'\n'],
        ];
        for original in samples {
            let d = decode_text(&original).unwrap();
            assert!(!d.text.contains('\r'));
            assert_eq!(encode_text(&d.text, d.encoding, d.crlf).unwrap(), original, "{:?}", d.encoding);
        }
        assert!(encode_text("€ 漢", TextEncoding::Windows1252, false).is_err());
    }
}
