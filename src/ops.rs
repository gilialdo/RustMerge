//! Operações de arquivo do menu de contexto: copiar, mover, apagar (para a Lixeira) e renomear.
//! Funcionam com arquivos e com pastas (de forma recursiva).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "esquerda",
            Side::Right => "direita",
        }
    }
}

/// Copia um arquivo ou uma pasta inteira, criando as pastas de destino e substituindo
/// arquivos que já existam. Pastas são mescladas (arquivos só do destino são mantidos).
pub fn copy_item(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_item(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        // No Windows o fs::copy mantém a data de modificação.
        fs::copy(src, dst).map(|_| ())
    }
}

/// Move um arquivo ou pasta. Usa renomear quando dá (mesmo disco, destino livre);
/// senão copia e apaga a origem.
pub fn move_item(src: &Path, dst: &Path) -> io::Result<()> {
    if !dst.exists() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::rename(src, dst).is_ok() {
            return Ok(());
        }
    }
    copy_item(src, dst)?;
    if src.is_dir() { fs::remove_dir_all(src) } else { fs::remove_file(src) }
}

/// Manda para a Lixeira.
pub fn delete_item(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| format!("Erro ao apagar {}: {e}", path.display()))
}

/// Verifica se o nome é válido para arquivo ou pasta no Windows.
pub fn validate_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("O nome não pode ficar vazio.".into());
    }
    if let Some(c) = name.chars().find(|c| r#"\/:*?"<>|"#.contains(*c) || c.is_control()) {
        return Err(format!("O nome não pode conter o caractere {c:?}."));
    }
    if name == "." || name == ".." || name.ends_with('.') {
        return Err("O nome não pode terminar com ponto.".into());
    }
    Ok(())
}

/// Renomeia mantendo a pasta. Devolve o novo caminho.
pub fn rename_item(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    validate_name(new_name)?;
    let target = path.with_file_name(new_name.trim());
    // Só troca maiúsculas/minúsculas: no Windows o "destino" é o próprio arquivo.
    let only_case = target.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase();
    if target.exists() && !only_case {
        return Err(format!("Já existe {}.", target.display()));
    }
    fs::rename(path, &target).map_err(|e| format!("Erro ao renomear {}: {e}", path.display()))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rustmerge_ops_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copia_e_move_pastas() {
        let root = temp_dir("copy");
        let src = root.join("l/pasta");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        fs::write(src.join("sub/b.txt"), "b").unwrap();

        // Copia para um lugar onde as pastas ainda não existem.
        let dst = root.join("r/x/pasta");
        copy_item(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("sub/b.txt")).unwrap(), "b");
        assert!(src.exists());

        // Copiar por cima substitui o arquivo.
        fs::write(src.join("a.txt"), "novo").unwrap();
        copy_item(&src.join("a.txt"), &dst.join("a.txt")).unwrap();
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "novo");

        // Mover para um destino que já existe: mescla e apaga a origem.
        move_item(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "novo");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn renomeia_e_valida_nome() {
        let root = temp_dir("rename");
        fs::write(root.join("a.txt"), "a").unwrap();
        fs::write(root.join("b.txt"), "b").unwrap();

        assert!(rename_item(&root.join("a.txt"), "b.txt").is_err());
        assert!(rename_item(&root.join("a.txt"), "c?.txt").is_err());
        assert!(rename_item(&root.join("a.txt"), "  ").is_err());
        let new = rename_item(&root.join("a.txt"), "c.txt").unwrap();
        assert_eq!(fs::read_to_string(new).unwrap(), "a");
        assert!(!root.join("a.txt").exists());
        // Só mudar maiúsculas/minúsculas é permitido.
        rename_item(&root.join("c.txt"), "C.txt").unwrap();

        fs::remove_dir_all(&root).unwrap();
    }
}
