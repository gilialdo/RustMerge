//! Mede o tempo da comparação de pastas.
//!
//! Gerar dados de teste (N arquivos em cada lado, ~10% diferentes só no último byte):
//!   cargo run --release --example bench -- gerar <pasta> <N>
//! Medir com 1, 2, 4... threads:
//!   cargo run --release --example bench -- medir <esquerda> <direita>

use std::fs;
use std::path::Path;
use std::time::Instant;

use rustmerge::folder::{CompareMethod, Options, Progress, compare_folders, cpu_count};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["gerar", dir, n] => generate(Path::new(dir), n.parse().expect("N inválido")),
        ["medir", left, right] => measure(Path::new(left), Path::new(right)),
        _ => eprintln!("uso: bench gerar <pasta> <N> | bench medir <esquerda> <direita>"),
    }
}

fn generate(dir: &Path, n: usize) {
    for side in ["esquerda", "direita"] {
        for i in 0..n {
            // 100 arquivos por subpasta, tamanhos entre 4 KB e ~132 KB.
            let sub = dir.join(side).join(format!("pasta{:04}", i / 100));
            fs::create_dir_all(&sub).unwrap();
            let size = 4096 + (i * 7919) % (128 * 1024);
            let mut data: Vec<u8> = (0..size).map(|j| ((i + j) % 251) as u8).collect();
            if side == "direita" && i % 10 == 0 {
                *data.last_mut().unwrap() ^= 0xFF;
            }
            fs::write(sub.join(format!("arquivo{i:06}.dat")), data).unwrap();
        }
    }
    println!("{n} arquivos gerados em cada lado de {}", dir.display());
}

fn measure(left: &Path, right: &Path) {
    let mut threads = vec![1, 2, 4];
    let max = cpu_count();
    while *threads.last().unwrap() * 2 <= max {
        threads.push(threads.last().unwrap() * 2);
    }
    if *threads.last().unwrap() != max {
        threads.push(max);
    }

    // Primeira execução só para aquecer o cache de disco do sistema.
    let _ = compare_folders(left, right, Options { method: CompareMethod::Content, threads: max, ..Options::default() }, &Progress::default());

    for method in [CompareMethod::Content, CompareMethod::SizeAndDate] {
        println!("\nMétodo: {}", method.description());
        let mut base = 0.0;
        for &t in &threads {
            let start = Instant::now();
            let cmp = compare_folders(left, right, Options { method, threads: t, ..Options::default() }, &Progress::default()).unwrap();
            let secs = start.elapsed().as_secs_f64();
            if t == 1 {
                base = secs;
            }
            let s = cmp.stats;
            println!(
                "  {t:>2} threads: {secs:>7.3} s  ({:.1}x)  iguais={} diferentes={}",
                base / secs,
                s.equal,
                s.different
            );
            if method == CompareMethod::SizeAndDate {
                break;
            }
        }
    }
}
