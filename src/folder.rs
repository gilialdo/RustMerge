//! Comparação de duas pastas em paralelo.
//!
//! Fase 1: lê as duas árvores de pastas em paralelo (uma tarefa por subpasta).
//! Fase 2: compara o conteúdo dos arquivos que existem nos dois lados, também em paralelo.
//! Arquivos com tamanhos diferentes já são marcados como diferentes sem serem abertos.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use rayon::prelude::*;

use crate::filter::ExtFilter;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Equal,
    Different,
    LeftOnly,
    RightOnly,
    Error,
}

impl Status {
    pub fn description(self) -> &'static str {
        match self {
            Status::Equal => "Idêntico",
            Status::Different => "Diferente",
            Status::LeftOnly => "Só na esquerda",
            Status::RightOnly => "Só na direita",
            Status::Error => "Erro ao ler",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CompareMethod {
    /// Tamanho e, se for igual, conteúdo byte a byte.
    #[default]
    Content,
    /// Só tamanho e data de modificação (não abre os arquivos).
    SizeAndDate,
}

impl CompareMethod {
    pub const ALL: [CompareMethod; 2] = [CompareMethod::Content, CompareMethod::SizeAndDate];

    pub fn description(self) -> &'static str {
        match self {
            CompareMethod::Content => "Conteúdo",
            CompareMethod::SizeAndDate => "Tamanho e data",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub method: CompareMethod,
    /// Número de threads; 0 = automático (`default_threads`).
    pub threads: usize,
    /// Não compara arquivos e pastas ocultos (e nada dentro das pastas ocultas).
    pub ignore_hidden: bool,
    /// Só compara arquivos com estas extensões. Pastas que ficam sem nenhum arquivo são escondidas.
    pub extensions: ExtFilter,
}

impl Default for Options {
    fn default() -> Self {
        Self { method: CompareMethod::Content, threads: 0, ignore_hidden: true, extensions: ExtFilter::default() }
    }
}

pub fn cpu_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

/// Threads usadas no modo automático. Nas medições, acima de 8 o disco e a memória
/// viram o gargalo e mais threads deixam a comparação mais lenta.
pub fn default_threads() -> usize {
    cpu_count().min(8)
}

/// Contadores lidos pela interface enquanto a comparação roda.
#[derive(Default)]
pub struct Progress {
    pub dirs_scanned: AtomicUsize,
    pub files_to_compare: AtomicUsize,
    pub files_compared: AtomicUsize,
    pub bytes_compared: AtomicU64,
    /// true quando a fase 2 (comparar conteúdo) começou.
    pub comparing: AtomicBool,
    pub cancel: AtomicBool,
}

impl Progress {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Item da árvore. Os nós ficam num vetor; `children` guarda índices desse vetor.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub rel_path: PathBuf,
    pub is_dir: bool,
    pub in_left: bool,
    pub in_right: bool,
    /// Tamanho e data na esquerda (só arquivos).
    pub left_info: Option<FileInfo>,
    /// Tamanho e data na direita (só arquivos).
    pub right_info: Option<FileInfo>,
    pub status: Status,
    pub depth: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

/// Contagem de arquivos (pastas não entram).
#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub equal: usize,
    pub different: usize,
    pub left_only: usize,
    pub right_only: usize,
    pub errors: usize,
}

pub struct Comparison {
    pub left_root: PathBuf,
    pub right_root: PathBuf,
    pub nodes: Vec<Node>,
    /// Itens do primeiro nível.
    pub roots: Vec<usize>,
    pub stats: Stats,
    pub elapsed: Duration,
}

impl Comparison {
    pub fn find(&self, rel_path: &Path) -> Option<usize> {
        self.nodes.iter().position(|n| n.rel_path == rel_path)
    }

    /// Recompara um arquivo (por exemplo, depois de editado) e atualiza as pastas acima dele
    /// e a contagem. Se o arquivo foi criado no lado onde faltava, as pastas também passam a
    /// existir nos dois lados.
    pub fn refresh_file(&mut self, index: usize) {
        let rel = self.nodes[index].rel_path.clone();
        let (left, right) = (self.left_root.join(&rel), self.right_root.join(&rel));
        let (in_left, in_right) = (left.is_file(), right.is_file());
        let status = match (in_left, in_right) {
            (true, true) => match files_equal(&left, &right) {
                Ok(true) => Status::Equal,
                Ok(false) => Status::Different,
                Err(_) => Status::Error,
            },
            (true, false) => Status::LeftOnly,
            (false, true) => Status::RightOnly,
            (false, false) => Status::Error,
        };
        let node = &mut self.nodes[index];
        (node.in_left, node.in_right, node.status) = (in_left, in_right, status);
        node.left_info = FileInfo::of_file(&left);
        node.right_info = FileInfo::of_file(&right);

        let mut parent = self.nodes[index].parent;
        while let Some(p) = parent {
            let rel = &self.nodes[p].rel_path;
            let in_left = self.left_root.join(rel).is_dir();
            let in_right = self.right_root.join(rel).is_dir();
            let status = match (in_left, in_right) {
                (true, true) if self.nodes[p].children.iter().all(|&c| self.nodes[c].status == Status::Equal) => {
                    Status::Equal
                }
                (true, true) => Status::Different,
                (true, false) => Status::LeftOnly,
                (false, true) => Status::RightOnly,
                (false, false) => Status::Error,
            };
            let node = &mut self.nodes[p];
            (node.in_left, node.in_right, node.status) = (in_left, in_right, status);
            parent = node.parent;
        }
        self.stats = count_files(&self.nodes);
    }
}

pub fn compare_folders(left: &Path, right: &Path, options: Options, progress: &Progress) -> Result<Comparison, String> {
    if !left.is_dir() {
        return Err(format!("Pasta da esquerda não encontrada: {}", left.display()));
    }
    if !right.is_dir() {
        return Err(format!("Pasta da direita não encontrada: {}", right.display()));
    }
    let start = Instant::now();
    let threads = if options.threads == 0 { default_threads() } else { options.threads };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|e| e.to_string())?;

    pool.install(|| {
        // Fase 1: estrutura das pastas.
        let tree = scan_dir(Some(left), Some(right), &options, progress);
        if progress.cancelled() {
            return Err("Comparação cancelada.".to_string());
        }

        let mut nodes = Vec::new();
        let mut pending = Vec::new();
        let roots = flatten(tree, Path::new(""), None, 0, options.method, &mut nodes, &mut pending);

        // Fase 2: conteúdo dos arquivos com o mesmo tamanho.
        progress.files_to_compare.store(pending.len(), Ordering::Relaxed);
        progress.comparing.store(true, Ordering::Relaxed);
        let results: Vec<(usize, Status)> = pending
            .par_iter()
            .map(|job| {
                if progress.cancelled() {
                    return (job.index, Status::Error);
                }
                let status = match contents_equal(&left.join(&job.rel_path), &right.join(&job.rel_path)) {
                    Ok(true) => Status::Equal,
                    Ok(false) => Status::Different,
                    Err(_) => Status::Error,
                };
                progress.files_compared.fetch_add(1, Ordering::Relaxed);
                progress.bytes_compared.fetch_add(job.size, Ordering::Relaxed);
                (job.index, status)
            })
            .collect();
        if progress.cancelled() {
            return Err("Comparação cancelada.".to_string());
        }
        for (index, status) in results {
            nodes[index].status = status;
        }

        update_dir_status(&mut nodes);
        let stats = count_files(&nodes);
        Ok(Comparison {
            left_root: left.to_path_buf(),
            right_root: right.to_path_buf(),
            nodes,
            roots,
            stats,
            elapsed: start.elapsed(),
        })
    })
}

/// Tamanho e data de modificação de um arquivo num dos lados.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl FileInfo {
    fn from_metadata(md: &fs::Metadata) -> Self {
        Self { size: md.len(), modified: md.modified().ok() }
    }

    /// Tamanho e data do arquivo no disco; `None` se não existir ou for pasta.
    fn of_file(path: &Path) -> Option<Self> {
        fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| Self::from_metadata(&m))
    }
}

struct ScanNode {
    name: String,
    is_dir: bool,
    left: Option<FileInfo>,
    right: Option<FileInfo>,
    children: Vec<ScanNode>,
}

struct Entry {
    name: String,
    is_dir: bool,
    meta: FileInfo,
}

/// Item oculto:
/// - pasta cujo nome começa com ponto (`.git`, `.vs`, `.idea`...), em qualquer sistema;
/// - no Windows, qualquer item com o atributo Oculto;
/// - nos outros sistemas, qualquer item cujo nome começa com ponto.
fn is_hidden(name: &str, md: Option<&fs::Metadata>) -> bool {
    let is_dir = md.is_some_and(|m| m.is_dir());
    if is_dir && name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        md.is_some_and(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
    }
    #[cfg(not(windows))]
    {
        name.starts_with('.')
    }
}

fn list_dir(dir: Option<&Path>, ignore_hidden: bool) -> Vec<Entry> {
    let Some(dir) = dir else { return Vec::new() };
    let Ok(read_dir) = fs::read_dir(dir) else { return Vec::new() };
    read_dir
        .filter_map(Result::ok)
        .filter_map(|e| {
            // No Windows o metadata do DirEntry vem da própria listagem, sem acesso extra ao disco.
            let md = e.metadata().ok();
            let name = e.file_name().to_string_lossy().into_owned();
            if ignore_hidden && is_hidden(&name, md.as_ref()) {
                return None;
            }
            Some(Entry {
                name,
                is_dir: md.as_ref().is_some_and(|m| m.is_dir()),
                meta: FileInfo {
                    size: md.as_ref().map_or(0, |m| m.len()),
                    modified: md.and_then(|m| m.modified().ok()),
                },
            })
        })
        .collect()
}

/// No Windows os nomes não diferenciam maiúsculas de minúsculas.
fn match_key(name: &str) -> String {
    if cfg!(windows) { name.to_lowercase() } else { name.to_string() }
}

fn scan_dir(left: Option<&Path>, right: Option<&Path>, options: &Options, progress: &Progress) -> Vec<ScanNode> {
    if progress.cancelled() {
        return Vec::new();
    }
    let hidden = options.ignore_hidden;
    let (left_entries, right_entries) = rayon::join(|| list_dir(left, hidden), || list_dir(right, hidden));
    progress.dirs_scanned.fetch_add(1, Ordering::Relaxed);

    let mut pairs: BTreeMap<String, (Option<Entry>, Option<Entry>)> = BTreeMap::new();
    for e in left_entries {
        let key = match_key(&e.name);
        pairs.entry(key).or_default().0 = Some(e);
    }
    for e in right_entries {
        let key = match_key(&e.name);
        pairs.entry(key).or_default().1 = Some(e);
    }

    // (nome, é pasta, metadados da esquerda, metadados da direita)
    let mut items: Vec<(String, bool, Option<FileInfo>, Option<FileInfo>)> = Vec::with_capacity(pairs.len());
    for (_, pair) in pairs {
        match pair {
            (Some(l), Some(r)) if l.is_dir == r.is_dir => items.push((l.name, l.is_dir, Some(l.meta), Some(r.meta))),
            // Mesmo nome, mas um lado é pasta e o outro é arquivo: viram dois itens.
            (Some(l), Some(r)) => {
                items.push((l.name, l.is_dir, Some(l.meta), None));
                items.push((r.name, r.is_dir, None, Some(r.meta)));
            }
            (Some(l), None) => items.push((l.name, l.is_dir, Some(l.meta), None)),
            (None, Some(r)) => items.push((r.name, r.is_dir, None, Some(r.meta))),
            (None, None) => {}
        }
    }

    // Filtro de extensões: arquivos que não passam nem chegam a ser comparados.
    let filtering = options.extensions.is_active();
    if filtering {
        items.retain(|(name, is_dir, _, _)| *is_dir || options.extensions.matches(name));
    }

    items
        .into_par_iter()
        .filter_map(|(name, is_dir, l, r)| {
            let children = if is_dir {
                let lp = l.and(left).map(|d| d.join(&name));
                let rp = r.and(right).map(|d| d.join(&name));
                scan_dir(lp.as_deref(), rp.as_deref(), options, progress)
            } else {
                Vec::new()
            };
            // Com filtro, pastas que ficaram sem nenhum arquivo são escondidas.
            if filtering && is_dir && children.is_empty() {
                return None;
            }
            Some(ScanNode { name, is_dir, left: l, right: r, children })
        })
        .collect()
}

struct ContentJob {
    index: usize,
    rel_path: PathBuf,
    size: u64,
}

/// Transforma a árvore lida em vetor de nós e lista os arquivos que precisam de comparação de conteúdo.
fn flatten(
    mut items: Vec<ScanNode>,
    parent: &Path,
    parent_index: Option<usize>,
    depth: usize,
    method: CompareMethod,
    nodes: &mut Vec<Node>,
    pending: &mut Vec<ContentJob>,
) -> Vec<usize> {
    items.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let mut indices = Vec::with_capacity(items.len());
    for item in items {
        let index = nodes.len();
        let rel_path = parent.join(&item.name);
        let status = match (item.left, item.right) {
            (Some(_), None) => Status::LeftOnly,
            (None, Some(_)) => Status::RightOnly,
            // Pastas nos dois lados: o status é calculado depois, a partir dos filhos.
            _ if item.is_dir => Status::Equal,
            (Some(l), Some(r)) => file_status_from_meta(l, r, method),
            (None, None) => Status::Error,
        };
        if status == Status::Equal && !item.is_dir && method == CompareMethod::Content {
            pending.push(ContentJob { index, rel_path: rel_path.clone(), size: item.left.map_or(0, |m| m.size) });
        }
        nodes.push(Node {
            name: item.name,
            rel_path: rel_path.clone(),
            is_dir: item.is_dir,
            in_left: item.left.is_some(),
            in_right: item.right.is_some(),
            // Tamanho e data só fazem sentido para arquivos.
            left_info: item.left.filter(|_| !item.is_dir),
            right_info: item.right.filter(|_| !item.is_dir),
            status,
            depth,
            parent: parent_index,
            children: Vec::new(),
        });
        let children = flatten(item.children, &rel_path, Some(index), depth + 1, method, nodes, pending);
        nodes[index].children = children;
        indices.push(index);
    }
    indices
}

/// Decide pelo tamanho/data. `Equal` no modo Conteúdo significa "precisa abrir e comparar".
fn file_status_from_meta(l: FileInfo, r: FileInfo, method: CompareMethod) -> Status {
    if l.size != r.size {
        return Status::Different;
    }
    match method {
        CompareMethod::Content => Status::Equal,
        CompareMethod::SizeAndDate => {
            // Tolerância de 2 s por causa de sistemas de arquivos como FAT.
            let same_date = match (l.modified, r.modified) {
                (Some(a), Some(b)) => {
                    let diff = a.duration_since(b).or_else(|_| b.duration_since(a)).unwrap_or_default();
                    diff <= Duration::from_secs(2)
                }
                _ => false,
            };
            if same_date { Status::Equal } else { Status::Different }
        }
    }
}

/// Pastas presentes nos dois lados ficam diferentes se algum filho for diferente.
/// Os filhos sempre têm índice maior que o pai, então basta percorrer de trás para frente.
fn update_dir_status(nodes: &mut [Node]) {
    for i in (0..nodes.len()).rev() {
        if nodes[i].is_dir && nodes[i].in_left && nodes[i].in_right {
            let all_equal = nodes[i].children.iter().all(|&c| nodes[c].status == Status::Equal);
            nodes[i].status = if all_equal { Status::Equal } else { Status::Different };
        }
    }
}

fn count_files(nodes: &[Node]) -> Stats {
    let mut s = Stats::default();
    for n in nodes.iter().filter(|n| !n.is_dir) {
        match n.status {
            Status::Equal => s.equal += 1,
            Status::Different => s.different += 1,
            Status::LeftOnly => s.left_only += 1,
            Status::RightOnly => s.right_only += 1,
            Status::Error => s.errors += 1,
        }
    }
    s
}

const BUFFER_SIZE: usize = 256 * 1024;

thread_local! {
    // Cada thread reaproveita os mesmos buffers em vez de alocar a cada arquivo.
    static BUFFERS: RefCell<(Vec<u8>, Vec<u8>)> = RefCell::new((vec![0; BUFFER_SIZE], vec![0; BUFFER_SIZE]));
}

fn open_sequential(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_SEQUENTIAL_SCAN: avisa o Windows que a leitura é sequencial.
        options.custom_flags(0x0800_0000);
    }
    options.open(path)
}

/// Compara pelo tamanho e, se for igual, byte a byte.
pub fn files_equal(a: &Path, b: &Path) -> io::Result<bool> {
    if fs::metadata(a)?.len() != fs::metadata(b)?.len() {
        return Ok(false);
    }
    contents_equal(a, b)
}

/// Compara byte a byte (o chamador já sabe que os tamanhos são iguais).
fn contents_equal(a: &Path, b: &Path) -> io::Result<bool> {
    BUFFERS.with(|buffers| {
        let (ba, bb) = &mut *buffers.borrow_mut();
        let mut fa = open_sequential(a)?;
        let mut fb = open_sequential(b)?;
        loop {
            let na = read_full(&mut fa, ba)?;
            let nb = read_full(&mut fb, bb)?;
            if na != nb || ba[..na] != bb[..nb] {
                return Ok(false);
            }
            if na < ba.len() {
                return Ok(true);
            }
        }
    })
}

fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..])? {
            0 => break,
            n => total += n,
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rustmerge_test_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample(root: &Path) -> (PathBuf, PathBuf) {
        let (l, r) = (root.join("l"), root.join("r"));
        fs::create_dir_all(l.join("sub")).unwrap();
        fs::create_dir_all(r.join("sub")).unwrap();
        fs::create_dir_all(l.join("so_esquerda")).unwrap();
        fs::write(l.join("igual.txt"), "abc").unwrap();
        fs::write(r.join("igual.txt"), "abc").unwrap();
        fs::write(l.join("sub/dif.txt"), "abc").unwrap();
        fs::write(r.join("sub/dif.txt"), "abd").unwrap();
        fs::write(l.join("so_esquerda/a.txt"), "x").unwrap();
        fs::write(r.join("novo.txt"), "y").unwrap();
        // Arquivo maior que o buffer, diferente só no último byte.
        let mut big = vec![7u8; BUFFER_SIZE * 2 + 10];
        fs::write(l.join("grande.bin"), &big).unwrap();
        *big.last_mut().unwrap() = 8;
        fs::write(r.join("grande.bin"), &big).unwrap();
        (l, r)
    }

    #[test]
    fn compara_pastas_em_paralelo() {
        let root = temp_dir("folders");
        let (l, r) = sample(&root);

        for threads in [1, 4] {
            let options = Options { method: CompareMethod::Content, threads, ..Options::default() };
            let cmp = compare_folders(&l, &r, options, &Progress::default()).unwrap();
            let s = cmp.stats;
            assert_eq!((s.equal, s.different, s.left_only, s.right_only), (1, 2, 1, 1));

            let find = |name: &str| &cmp.nodes[*cmp.roots.iter().find(|&&i| cmp.nodes[i].name == name).unwrap()];
            assert_eq!(find("sub").status, Status::Different);
            assert_eq!(find("so_esquerda").status, Status::LeftOnly);
            assert_eq!(find("novo.txt").status, Status::RightOnly);
            assert_eq!(find("igual.txt").status, Status::Equal);
            assert_eq!(find("grande.bin").status, Status::Different);
            // Tamanho e data guardados para arquivos; pastas ficam sem.
            assert_eq!(find("igual.txt").left_info.unwrap().size, 3);
            assert!(find("igual.txt").right_info.unwrap().modified.is_some());
            assert!(find("novo.txt").left_info.is_none());
            assert!(find("sub").left_info.is_none());
            // Pastas vêm antes dos arquivos.
            assert!(cmp.nodes[cmp.roots[0]].is_dir);
        }
        fs::remove_dir_all(&root).unwrap();
    }

    /// Cria um arquivo ou pasta oculto (atributo no Windows, nome com ponto nos outros sistemas).
    fn make_hidden(parent: &Path, name: &str, dir: bool) {
        let name = if cfg!(windows) { name.to_string() } else { format!(".{name}") };
        let path = parent.join(name);
        if dir {
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("dentro.txt"), "z").unwrap();
        } else {
            fs::write(&path, "oculto").unwrap();
        }
        #[cfg(windows)]
        {
            let ok = std::process::Command::new("attrib").arg("+h").arg(&path).status().unwrap().success();
            assert!(ok, "attrib +h falhou");
        }
    }

    #[test]
    fn ignora_ocultos() {
        let root = temp_dir("hidden");
        let (l, r) = sample(&root);
        make_hidden(&l, "oculto.txt", false);
        make_hidden(&r, "pasta_oculta", true);
        // Pasta com ponto (sem atributo Oculto) e arquivo com ponto.
        fs::create_dir_all(l.join(".git")).unwrap();
        fs::write(l.join(".git/config"), "c").unwrap();
        fs::write(l.join(".gitignore"), "target").unwrap();

        let names = |ignore_hidden: bool| {
            let options = Options { ignore_hidden, ..Options::default() };
            let cmp = compare_folders(&l, &r, options, &Progress::default()).unwrap();
            cmp.nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>()
        };
        let has = |list: &[String], name: &str| list.iter().any(|n| n.trim_start_matches('.') == name.trim_start_matches('.'));

        let ignoring = names(true);
        for hidden in ["oculto.txt", "pasta_oculta", "dentro.txt", ".git", "config"] {
            assert!(!has(&ignoring, hidden), "{hidden} deveria ser ignorado");
        }
        // No Windows, arquivo com ponto não é oculto (só pastas com ponto são).
        if cfg!(windows) {
            assert!(has(&ignoring, ".gitignore"));
        }

        let showing = names(false);
        for name in ["oculto.txt", "pasta_oculta", "dentro.txt", ".git", "config", ".gitignore"] {
            assert!(has(&showing, name), "{name} deveria aparecer");
        }

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn recompara_arquivo_editado() {
        let root = temp_dir("refresh");
        let (l, r) = sample(&root);
        let mut cmp = compare_folders(&l, &r, Options::default(), &Progress::default()).unwrap();
        let idx = |cmp: &Comparison, p: &str| cmp.find(Path::new(p)).unwrap();
        assert_eq!(cmp.stats.different, 2);

        // Edita o arquivo da direita para ficar igual: o arquivo e a pasta "sub" ficam iguais.
        fs::write(r.join("sub/dif.txt"), "abc").unwrap();
        let i = idx(&cmp, "sub/dif.txt");
        cmp.refresh_file(i);
        assert_eq!(cmp.nodes[i].status, Status::Equal);
        assert_eq!(cmp.nodes[idx(&cmp, "sub")].status, Status::Equal);
        assert_eq!(cmp.stats.different, 1);

        // Salva no lado onde o arquivo não existia: a pasta passa a existir nos dois lados.
        fs::create_dir_all(r.join("so_esquerda")).unwrap();
        fs::write(r.join("so_esquerda/a.txt"), "x").unwrap();
        let i = idx(&cmp, "so_esquerda/a.txt");
        assert!(cmp.nodes[i].right_info.is_none());
        cmp.refresh_file(i);
        assert_eq!(cmp.nodes[i].status, Status::Equal);
        assert_eq!(cmp.nodes[i].right_info.unwrap().size, 1);
        let dir = &cmp.nodes[idx(&cmp, "so_esquerda")];
        assert!(dir.in_left && dir.in_right);
        assert_eq!(dir.status, Status::Equal);
        assert_eq!(cmp.stats.left_only, 0);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn filtra_por_extensao() {
        let root = temp_dir("ext");
        let (l, r) = sample(&root);
        // Pasta só com .bin: deve sumir quando o filtro é "txt".
        fs::create_dir_all(l.join("so_bin")).unwrap();
        fs::write(l.join("so_bin/x.bin"), "b").unwrap();

        let options = Options { extensions: ExtFilter::parse("txt"), ..Options::default() };
        let cmp = compare_folders(&l, &r, options, &Progress::default()).unwrap();
        let names: Vec<&str> = cmp.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"igual.txt"));
        assert!(names.contains(&"dif.txt"));
        assert!(!names.contains(&"grande.bin"));
        assert!(!names.contains(&"so_bin"));
        assert_eq!(cmp.stats.different, 1);

        // Exclusão: "!txt" deixa só os outros.
        let options = Options { extensions: ExtFilter::parse("!txt"), ..Options::default() };
        let cmp = compare_folders(&l, &r, options, &Progress::default()).unwrap();
        assert!(cmp.nodes.iter().all(|n| n.is_dir || !n.name.ends_with(".txt")));
        assert!(cmp.nodes.iter().any(|n| n.name == "grande.bin"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cancela_comparacao() {
        let root = temp_dir("cancel");
        let (l, r) = sample(&root);
        let progress = Progress::default();
        progress.cancel.store(true, Ordering::Relaxed);
        assert!(compare_folders(&l, &r, Options::default(), &progress).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
