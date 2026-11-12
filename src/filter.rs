//! Filtro de extensões de arquivo.
//!
//! Texto aceito: extensões separadas por vírgula, ponto e vírgula ou espaço, com ou sem
//! `*.` / `.` na frente (`rs, .toml *.txt`). Com `!` na frente, a extensão é excluída (`!log`).
//! Sem nenhuma extensão de inclusão, todos os arquivos passam (menos os excluídos).

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtFilter {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl ExtFilter {
    pub fn parse(text: &str) -> Self {
        let mut filter = Self::default();
        for raw in text.split([',', ';', ' ', '\t']) {
            let raw = raw.trim();
            let (excluded, rest) = match raw.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, raw),
            };
            let ext = rest.trim_start_matches('*').trim_start_matches('.').to_lowercase();
            if ext.is_empty() {
                continue;
            }
            if excluded { filter.exclude.push(ext) } else { filter.include.push(ext) }
        }
        filter
    }

    /// Há algum filtro ativo?
    pub fn is_active(&self) -> bool {
        !self.include.is_empty() || !self.exclude.is_empty()
    }

    /// O arquivo passa pelo filtro? (Compara o fim do nome, então `tar.gz` também funciona.)
    pub fn matches(&self, file_name: &str) -> bool {
        let name = file_name.to_lowercase();
        let has = |ext: &String| name.len() > ext.len() && name.ends_with(ext.as_str()) && name[..name.len() - ext.len()].ends_with('.');
        if self.exclude.iter().any(has) {
            return false;
        }
        self.include.is_empty() || self.include.iter().any(has)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtra_extensoes() {
        let f = ExtFilter::parse("rs, .TOML;*.txt  tar.gz");
        assert!(f.is_active());
        assert!(f.matches("main.rs"));
        assert!(f.matches("Cargo.toml"));
        assert!(f.matches("NOTAS.TXT"));
        assert!(f.matches("backup.tar.gz"));
        assert!(!f.matches("imagem.png"));
        assert!(!f.matches("rs"));
        assert!(!f.matches("arquivors"));

        let f = ExtFilter::parse("!log !tmp");
        assert!(f.matches("main.rs"));
        assert!(!f.matches("app.LOG"));
        assert!(!f.matches("x.tmp"));

        let f = ExtFilter::parse("  , ; ");
        assert!(!f.is_active());
        assert!(f.matches("qualquer.coisa"));
    }
}
