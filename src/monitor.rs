//! Consumo de recursos *solo de este proceso*: CPU, RAM, GPU y VRAM.
//!
//! Se mide en un hilo propio, una vez por segundo, y solo mientras el monitor
//! está visible (al ocultarlo el hilo termina). Sin crates extra: llamadas
//! directas a la API de Windows.
//! - CPU: `GetProcessTimes` (normalizado al total de núcleos, como el Administrador de tareas).
//! - RAM: `GetProcessMemoryInfo` → memoria privada (lo que realmente "gasta" la app).
//! - GPU/VRAM: contadores PDH `GPU Engine` / `GPU Process Memory` filtrados por PID.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Stats {
    /// CPU en centésimas de % (123 = 1.23 %).
    cpu_centi: AtomicU32,
    ram_bytes: AtomicU64,
    gpu_centi: AtomicU32,
    vram_bytes: AtomicU64,
    gpu_available: AtomicBool,
    ready: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    pub cpu_percent: f32,
    pub ram_mb: f32,
    /// `None` si el sistema no expone contadores de GPU.
    pub gpu_percent: Option<f32>,
    pub vram_mb: Option<f32>,
}

pub struct Monitor {
    stats: Arc<Stats>,
    stop: Arc<AtomicBool>,
}

impl Monitor {
    pub fn start(on_sample: impl Fn() + Send + 'static) -> Self {
        let stats = Arc::new(Stats::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (stats.clone(), stop.clone());
        let _ = std::thread::Builder::new()
            .name("monitor".into())
            .stack_size(64 * 1024)
            .spawn(move || sampler(s, st, on_sample));
        Self { stats, stop }
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        let s = &self.stats;
        if !s.ready.load(Ordering::Relaxed) {
            return None;
        }
        let gpu = s.gpu_available.load(Ordering::Relaxed);
        Some(Snapshot {
            cpu_percent: s.cpu_centi.load(Ordering::Relaxed) as f32 / 100.0,
            ram_mb: s.ram_bytes.load(Ordering::Relaxed) as f32 / 1_048_576.0,
            gpu_percent: gpu.then(|| s.gpu_centi.load(Ordering::Relaxed) as f32 / 100.0),
            vram_mb: gpu.then(|| s.vram_bytes.load(Ordering::Relaxed) as f32 / 1_048_576.0),
        })
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(windows)]
fn sampler(stats: Arc<Stats>, stop: Arc<AtomicBool>, on_sample: impl Fn()) {
    let mut cpu = win::CpuMeter::new();
    let gpu = win::GpuMeter::new(std::process::id());
    stats.gpu_available.store(gpu.is_some(), Ordering::Relaxed);
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(INTERVAL);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        stats
            .cpu_centi
            .store((cpu.sample() * 100.0) as u32, Ordering::Relaxed);
        stats
            .ram_bytes
            .store(win::private_bytes(), Ordering::Relaxed);
        if let Some(g) = &gpu {
            if let Some((util, vram)) = g.sample() {
                stats
                    .gpu_centi
                    .store((util * 100.0) as u32, Ordering::Relaxed);
                stats.vram_bytes.store(vram, Ordering::Relaxed);
            }
        }
        stats.ready.store(true, Ordering::Relaxed);
        on_sample();
    }
}

#[cfg(not(windows))]
fn sampler(_stats: Arc<Stats>, _stop: Arc<AtomicBool>, _on_sample: impl Fn()) {}

#[cfg(windows)]
mod win {
    use std::collections::HashMap;
    use std::time::Instant;

    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
        PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
        PDH_MORE_DATA,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    pub fn private_bytes() -> u64 {
        // SAFETY: estructura POD inicializada a cero y tamaño correcto en `cb`.
        unsafe {
            let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
            c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            let ok = GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
                c.cb,
            );
            if ok != 0 {
                c.PrivateUsage as u64
            } else {
                0
            }
        }
    }

    fn ft(f: FILETIME) -> u64 {
        ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
    }

    fn process_cpu_100ns() -> u64 {
        // SAFETY: punteros a FILETIME locales válidos.
        unsafe {
            let z = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let (mut c, mut e, mut k, mut u) = (z, z, z, z);
            if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) != 0 {
                ft(k) + ft(u)
            } else {
                0
            }
        }
    }

    pub struct CpuMeter {
        last_cpu: u64,
        last_t: Instant,
        cores: f64,
    }

    impl CpuMeter {
        pub fn new() -> Self {
            Self {
                last_cpu: process_cpu_100ns(),
                last_t: Instant::now(),
                cores: std::thread::available_parallelism().map_or(1, |n| n.get()) as f64,
            }
        }

        /// % de CPU del proceso respecto a toda la máquina desde la última muestra.
        pub fn sample(&mut self) -> f32 {
            let now = Instant::now();
            let cpu = process_cpu_100ns();
            let wall_100ns = now.duration_since(self.last_t).as_nanos() as f64 / 100.0;
            let used = cpu.saturating_sub(self.last_cpu) as f64;
            self.last_cpu = cpu;
            self.last_t = now;
            if wall_100ns <= 0.0 {
                return 0.0;
            }
            (used / wall_100ns / self.cores * 100.0).clamp(0.0, 100.0) as f32
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct GpuMeter {
        // Alias de windows-sys: eran `isize` hasta la 0.59 y son punteros desde la 0.60.
        query: PDH_HQUERY,
        engines: PDH_HCOUNTER,
        memory: PDH_HCOUNTER,
    }

    // Los handles de PDH solo se usan desde el hilo del monitor.
    unsafe impl Send for GpuMeter {}

    impl GpuMeter {
        pub fn new(pid: u32) -> Option<Self> {
            // SAFETY: llamadas FFI con punteros a variables locales válidas.
            unsafe {
                let mut query: PDH_HQUERY = std::ptr::null_mut();
                if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                    return None;
                }
                let mut engines: PDH_HCOUNTER = std::ptr::null_mut();
                let mut memory: PDH_HCOUNTER = std::ptr::null_mut();
                let p1 = wide(&format!(r"\GPU Engine(pid_{pid}_*)\Utilization Percentage"));
                let p2 = wide(&format!(
                    r"\GPU Process Memory(pid_{pid}_*)\Dedicated Usage"
                ));
                if PdhAddEnglishCounterW(query, p1.as_ptr(), 0, &mut engines) != 0
                    || PdhAddEnglishCounterW(query, p2.as_ptr(), 0, &mut memory) != 0
                {
                    PdhCloseQuery(query);
                    return None;
                }
                // Primera muestra: los contadores de tasa necesitan dos lecturas.
                PdhCollectQueryData(query);
                Some(Self {
                    query,
                    engines,
                    memory,
                })
            }
        }

        /// (% de uso de GPU, bytes de VRAM dedicada).
        pub fn sample(&self) -> Option<(f32, u64)> {
            // SAFETY: handles válidos mientras vive `self`.
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
            }
            // Como el Administrador de tareas: se suma por tipo de motor (3D,
            // Copy, VideoDecode…) y se muestra el motor más ocupado.
            let mut by_engine: HashMap<String, f64> = HashMap::new();
            for (name, value) in read_array(self.engines) {
                let kind = name.rsplit("engtype_").next().unwrap_or("").to_string();
                *by_engine.entry(kind).or_default() += value;
            }
            let util = by_engine
                .values()
                .cloned()
                .fold(0.0, f64::max)
                .clamp(0.0, 100.0);
            let vram: f64 = read_array(self.memory).into_iter().map(|(_, v)| v).sum();
            Some((util as f32, vram.max(0.0) as u64))
        }
    }

    impl Drop for GpuMeter {
        fn drop(&mut self) {
            // SAFETY: cerramos el query que abrimos.
            unsafe {
                PdhCloseQuery(self.query);
            }
        }
    }

    fn read_array(counter: PDH_HCOUNTER) -> Vec<(String, f64)> {
        // SAFETY: protocolo estándar de PDH (pedir tamaño, reservar, leer); el
        // buffer se reserva alineado a la estructura de items.
        unsafe {
            let (mut size, mut count) = (0u32, 0u32);
            let r = PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                std::ptr::null_mut(),
            );
            if r != PDH_MORE_DATA || size == 0 {
                return Vec::new();
            }
            let item = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> =
                Vec::with_capacity((size as usize).div_ceil(item));
            let items = buf.as_mut_ptr();
            if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, items)
                != 0
            {
                return Vec::new();
            }
            (0..count as usize)
                .map(|i| {
                    let it = &*items.add(i);
                    let mut len = 0;
                    while *it.szName.add(len) != 0 {
                        len += 1;
                    }
                    let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, len));
                    let v = if it.FmtValue.CStatus == 0 {
                        it.FmtValue.Anonymous.doubleValue
                    } else {
                        0.0
                    };
                    (name, v)
                })
                .collect()
        }
    }
}
