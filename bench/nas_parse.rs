//! NAS パースの壁時計・割り当て比較。`cargo bench --bench nas_parse`
//!
//! 測り方と妥当性の限界は `bench/nas_parse.md`。

use std::alloc::{GlobalAlloc, Layout, System};
use std::fs::File;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use memmap2::Mmap;
use meshpad::gpu::SceneGpu;
use meshpad::load;
use meshpad::nas;

const WARMUP: usize = 1;
const RUNS: usize = 5;
const HUGE_TRIS: usize = 10_000_000;

struct AllocCounter {
    allocated: AtomicUsize,
    current: AtomicUsize,
    peak: AtomicUsize,
}

#[global_allocator]
static ALLOC: AllocCounter = AllocCounter {
    allocated: AtomicUsize::new(0),
    current: AtomicUsize::new(0),
    peak: AtomicUsize::new(0),
};

unsafe impl GlobalAlloc for AllocCounter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            self.add(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        self.sub(layout.size());
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            self.add(layout.size());
        }
        p
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            if new_size >= layout.size() {
                self.add(new_size - layout.size());
            } else {
                self.sub(layout.size() - new_size);
            }
        }
        p
    }
}

impl AllocCounter {
    fn add(&self, n: usize) {
        self.allocated.fetch_add(n, Ordering::Relaxed);
        let cur = self.current.fetch_add(n, Ordering::Relaxed) + n;
        self.peak.fetch_max(cur, Ordering::Relaxed);
    }

    fn sub(&self, n: usize) {
        self.current.fetch_sub(n, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy)]
struct Sample {
    ms: f64,
    peak_mb: f64,
    alloc_mb: f64,
}

fn main() {
    let gpu = match gpu_device() {
        Ok(pair) => {
            eprintln!("gpu: dx12 device ready");
            Some(pair)
        }
        Err(e) => {
            eprintln!("gpu: skipped ({e})");
            None
        }
    };

    let mut files = default_files();
    files.extend(std::env::args().skip(1).map(PathBuf::from));
    files.retain(|p| p.is_file());
    if files.is_empty() {
        eprintln!("no NAS files (generate bench/data/derived first)");
        std::process::exit(1);
    }

    let py = pynastran_python();
    if let Some(p) = &py {
        eprintln!("pynas: {}", p.display());
    } else {
        eprintln!("pynas: skipped (no bench/.venv)");
    }

    println!(
        "{:<42} {:>8} {:>10} {:>11} {:>11} {:>11} {:>11}",
        "file", "MB", "skin_tris", "parse", "soup", "gpu", "pynas"
    );

    let mut rows = Vec::new();
    for path in files {
        match row(&path, gpu.as_ref(), py.as_deref()) {
            Ok(r) => {
                println!("{}", r.time_line);
                rows.push(r);
            }
            Err(e) => eprintln!("{}: {e}", path.display()),
        }
    }

    println!();
    println!(
        "{:<42} {:>11} {:>11} {:>11}",
        "peak extra MB", "parse", "soup", "gpu"
    );
    for r in &rows {
        println!("{}", r.peak_line);
    }

    println!();
    println!(
        "{:<42} {:>11} {:>11} {:>11}",
        "allocated MB", "parse", "soup", "gpu"
    );
    for r in &rows {
        println!("{}", r.alloc_line);
    }
}

struct RowOut {
    time_line: String,
    peak_line: String,
    alloc_line: String,
}

fn default_files() -> Vec<PathBuf> {
    [
        "bench/data/derived/nas_surface/bunny_res3_small8.nas",
        "bench/data/derived/nas_surface/bunny_res3_large16.nas",
        "bench/data/derived/nas_surface/bunny_small8.nas",
        "bench/data/derived/nas_surface/bunny_large16.nas",
        "bench/data/derived/nas_surface/happy_res2_small8.nas",
        "bench/data/derived/nas_surface/happy_small8.nas",
        "bench/data/derived/nas_volume/box_hex_c20_small8.nas",
        "bench/data/derived/nas_volume/box_hex_c20_large16.nas",
        "bench/data/derived/nas_volume/box_tet_c20_small8.nas",
        "bench/data/derived/nas_volume/box_hex_c40_small8.nas",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

fn row(
    path: &Path,
    gpu: Option<&(wgpu::Device, wgpu::Queue)>,
    py: Option<&Path>,
) -> anyhow::Result<RowOut> {
    let meta = std::fs::metadata(path)?;
    let mb = meta.len() as f64 / (1024.0 * 1024.0);
    let mmap = map(path)?;
    let (soup0, _) = nas::parse_nas(&mmap)?;
    let tris = soup0.triangle_count();
    drop(soup0);
    let runs = if tris >= HUGE_TRIS { 3 } else { RUNS };

    let parse = median(
        || {
            let (s, _) = nas::parse_nas(&mmap).unwrap();
            black_box(s.triangle_count());
        },
        runs,
    );
    let soup = median(
        || {
            let (s, _) = load::load_paths(&[path]).unwrap();
            black_box(s.triangle_count());
        },
        runs,
    );

    let gpu_s = match gpu {
        Some((device, _))
            if (tris as u64).saturating_mul(36) <= device.limits().max_buffer_size =>
        {
            let (mesh, _) = load::load_paths(&[path]).unwrap();
            Some(median(
                || {
                    let scene = SceneGpu::from_soup(device, &mesh);
                    black_box(scene.radius);
                    drop(scene);
                },
                runs,
            ))
        }
        Some(_) => None,
        None => None,
    };

    let pynas_runs = if mb > 40.0 { 3 } else { runs };
    let pynas = py.and_then(|exe| pynas_read(exe, path, pynas_runs));

    let label = short_label(path);
    let time_line = format!(
        "{label:<42} {mb:>8.2} {tris:>10} {:>10} {:>10} {:>10} {:>10}",
        fmt_ms(parse.ms),
        fmt_ms(soup.ms),
        gpu_s
            .as_ref()
            .map(|s| fmt_ms(s.ms))
            .unwrap_or_else(|| "-".into()),
        pynas
            .as_ref()
            .map(|s| fmt_ms(s.ms))
            .unwrap_or_else(|| "-".into()),
    );
    let peak_line = format!(
        "{label:<42} {:>11.1} {:>11.1} {:>11}",
        parse.peak_mb,
        soup.peak_mb,
        gpu_s
            .as_ref()
            .map(|s| format!("{:>11.1}", s.peak_mb))
            .unwrap_or_else(|| format!("{:>11}", "-")),
    );
    let alloc_line = format!(
        "{label:<42} {:>11.1} {:>11.1} {:>11}",
        parse.alloc_mb,
        soup.alloc_mb,
        gpu_s
            .as_ref()
            .map(|s| format!("{:>11.1}", s.alloc_mb))
            .unwrap_or_else(|| format!("{:>11}", "-")),
    );
    Ok(RowOut {
        time_line,
        peak_line,
        alloc_line,
    })
}

fn pynastran_python() -> Option<PathBuf> {
    ["bench/.venv/Scripts/python.exe", "bench/.venv/bin/python"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

fn pynas_read(py: &Path, nas: &Path, runs: usize) -> Option<Sample> {
    let script = Path::new("bench/scripts/nas_pynastran_read.py");
    if !script.is_file() {
        return None;
    }
    let out = std::process::Command::new(py)
        .arg(script)
        .arg(nas)
        .arg(runs.to_string())
        .output()
        .ok()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if !err.trim().is_empty() {
            eprint!("pynas {}: {err}", nas.display());
        }
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let ms: f64 = line.split_whitespace().next()?.parse().ok()?;
    Some(Sample {
        ms,
        peak_mb: 0.0,
        alloc_mb: 0.0,
    })
}

fn gpu_device() -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .map_err(|e| anyhow::anyhow!("no dx12 adapter: {e}"))?;
    let limits = adapter.limits();
    eprintln!(
        "gpu: adapter max_buffer_size = {:.0} MB",
        limits.max_buffer_size as f64 / (1024.0 * 1024.0)
    );
    let desc = wgpu::DeviceDescriptor {
        label: Some("nas_parse"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    };
    Ok(pollster::block_on(adapter.request_device(&desc))?)
}

fn map(path: &Path) -> anyhow::Result<Mmap> {
    let file = File::open(path)?;
    Ok(unsafe { Mmap::map(&file)? })
}

fn median(mut f: impl FnMut(), runs: usize) -> Sample {
    for _ in 0..WARMUP {
        f();
    }
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        samples.push(measure(&mut f));
    }
    samples.sort_by(|a, b| a.ms.partial_cmp(&b.ms).unwrap());
    let mid = samples[samples.len() / 2];
    let mut peaks: Vec<_> = samples.iter().map(|s| s.peak_mb).collect();
    peaks.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut allocs: Vec<_> = samples.iter().map(|s| s.alloc_mb).collect();
    allocs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Sample {
        ms: mid.ms,
        peak_mb: peaks[peaks.len() / 2],
        alloc_mb: allocs[allocs.len() / 2],
    }
}

fn measure(f: &mut impl FnMut()) -> Sample {
    let cur0 = ALLOC.current.load(Ordering::Relaxed);
    let alloc0 = ALLOC.allocated.load(Ordering::Relaxed);
    ALLOC.peak.store(cur0, Ordering::Relaxed);
    let t = Instant::now();
    f();
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let peak = ALLOC.peak.load(Ordering::Relaxed).saturating_sub(cur0);
    let allocated = ALLOC
        .allocated
        .load(Ordering::Relaxed)
        .saturating_sub(alloc0);
    Sample {
        ms,
        peak_mb: bytes_mb(peak),
        alloc_mb: bytes_mb(allocated),
    }
}

fn bytes_mb(n: usize) -> f64 {
    n as f64 / (1024.0 * 1024.0)
}

fn fmt_ms(ms: f64) -> String {
    if ms < 10.0 {
        format!("{ms:.2}ms")
    } else {
        format!("{ms:.1}ms")
    }
}

fn short_label(path: &Path) -> String {
    let mut parts = path.iter().rev().take(2).collect::<Vec<_>>();
    parts.reverse();
    parts
        .into_iter()
        .map(|s| s.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
