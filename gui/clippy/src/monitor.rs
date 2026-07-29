//! What the machine and the session are costing, sampled from the kernel.
//!
//! Two shapes, because the two reference tools are two shapes. radeontop is a
//! list of labelled bars, each a value against a maximum. btop is the same
//! values sampled repeatedly and drawn as a rolling graph. Both fall out of one
//! [`Gauge`]: a `max` means the value is a proportion and can be a bar, and the
//! history behind it is a series and can be a graph.
//!
//! Everything is read from `/sys` and `/proc` as text. No dependency, no
//! vendor library, and nothing that fails harder than reporting one fewer
//! reading: a machine without amdgpu simply has no GPU rows.
//!
//! Sampling only runs while this view is on screen. A monitor is inherently
//! periodic and periodic is the opposite of the redraw-on-change rule, so the
//! rule is kept by not sampling when nobody is looking.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;

/// One measured quantity. `max` present means it can be drawn as a bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Gauge {
    pub key: &'static str,
    pub label: &'static str,
    pub value: f64,
    pub max: Option<f64>,
    pub unit: &'static str,
}

impl Gauge {
    /// Where the bar fills to, 0.0 to 1.0. Unbounded readings have no bar.
    pub fn fraction(&self) -> Option<f32> {
        match self.max {
            Some(max) if max > 0.0 => Some((self.value / max).clamp(0.0, 1.0) as f32),
            _ => None,
        }
    }

    /// The reading, written the way the unit wants it.
    pub fn reading(&self) -> String {
        match (self.unit, self.max) {
            ("%", _) => format!("{:.0}%", self.value),
            ("MiB", Some(max)) => format!("{:.0} / {:.0} MiB", self.value, max),
            ("MiB", None) => format!("{:.0} MiB", self.value),
            ("tok", Some(max)) => format!("{:.0} / {:.0}", self.value, max),
            ("tok", None) => format!("{:.0}", self.value),
            ("tok/s", _) if self.value <= 0.0 => String::from("—"),
            ("tok/s", _) => format!("{:.1} tok/s", self.value),
            ("", _) => format!("{:.0}", self.value),
            (unit, _) => format!("{:.1} {unit}", self.value),
        }
    }
}

const HISTORY: usize = 240;

pub struct Monitor {
    /// The amdgpu device directory, when there is one.
    gpu: Option<PathBuf>,
    /// Total and idle jiffies from the previous `/proc/stat` read, so the
    /// percentage is over the interval rather than since boot.
    cpu_prev: Option<(u64, u64)>,
    hardware: Vec<Gauge>,
    llm: Vec<Gauge>,
    history: HashMap<&'static str, VecDeque<f32>>,
}

impl Default for Monitor {
    fn default() -> Monitor {
        Monitor::new()
    }
}

impl Monitor {
    pub fn new() -> Monitor {
        Monitor {
            gpu: find_gpu(),
            cpu_prev: None,
            hardware: Vec::new(),
            llm: Vec::new(),
            history: HashMap::new(),
        }
    }

    /// What the machine is doing.
    pub fn hardware(&self) -> Vec<Gauge> {
        self.hardware.clone()
    }

    /// What the session is doing. Separate from the hardware because they are
    /// two different questions: one is whether the machine is keeping up, the
    /// other is whether the budget is.
    pub fn llm(&self) -> Vec<Gauge> {
        self.llm.clone()
    }

    /// Everything recorded for one key, oldest first. Empty until sampled.
    pub fn history(&self, key: &str) -> &[f32] {
        self.history
            .get(key)
            .map(|q| q.as_slices().0)
            .unwrap_or(&[])
    }

    /// Read every source once. Cheap: six small files, no allocation past the
    /// strings they contain.
    pub fn sample(&mut self, session: &crate::state::State) {
        let mut gauges = Vec::new();
        let mut llm = Vec::new();

        if let Some(gpu) = &self.gpu {
            if let Some(busy) = read_number(&gpu.join("gpu_busy_percent")) {
                gauges.push(Gauge {
                    key: "gpu",
                    label: "GPU",
                    value: busy,
                    max: Some(100.0),
                    unit: "%",
                });
            }
            if let (Some(used), Some(total)) = (
                read_number(&gpu.join("mem_info_vram_used")),
                read_number(&gpu.join("mem_info_vram_total")),
            ) {
                gauges.push(Gauge {
                    key: "vram",
                    label: "VRAM",
                    value: used / 1_048_576.0,
                    max: Some(total / 1_048_576.0),
                    unit: "MiB",
                });
            }
            if let (Some(used), Some(total)) = (
                read_number(&gpu.join("mem_info_gtt_used")),
                read_number(&gpu.join("mem_info_gtt_total")),
            ) {
                gauges.push(Gauge {
                    key: "gtt",
                    label: "GTT",
                    value: used / 1_048_576.0,
                    max: Some(total / 1_048_576.0),
                    unit: "MiB",
                });
            }
        }

        if let Some(stat) = read_text("/proc/stat")
            && let Some((total, idle)) = parse_cpu(&stat)
        {
            if let Some((prev_total, prev_idle)) = self.cpu_prev {
                let d_total = total.saturating_sub(prev_total) as f64;
                let d_idle = idle.saturating_sub(prev_idle) as f64;
                if d_total > 0.0 {
                    gauges.push(Gauge {
                        key: "cpu",
                        label: "CPU",
                        value: (1.0 - d_idle / d_total) * 100.0,
                        max: Some(100.0),
                        unit: "%",
                    });
                }
            }
            self.cpu_prev = Some((total, idle));
        }

        if let Some(meminfo) = read_text("/proc/meminfo")
            && let Some((total, available)) = parse_meminfo(&meminfo)
        {
            gauges.push(Gauge {
                key: "ram",
                label: "RAM",
                value: (total - available) / 1024.0,
                max: Some(total / 1024.0),
                unit: "MiB",
            });
        }

        // The session's economy, which is a different question from whether
        // the machine is keeping up: this is the budget that runs out first.
        // The agent's own reading where it sent one: it moves at every
        // transcript boundary, while usage only reports the request that
        // already went out. Falling back keeps a stream without measurements
        // showing something true rather than nothing.
        match (session.context, session.usage) {
            (Some(fill), _) if fill.total > 0 => llm.push(Gauge {
                key: "context",
                label: "CONTEXT",
                value: fill.used as f64,
                max: Some(fill.total as f64),
                unit: "tok",
            }),
            (_, Some(usage)) => llm.push(Gauge {
                key: "context",
                label: "CONTEXT",
                value: usage.prompt as f64,
                max: Some(usage.context_total as f64),
                unit: "tok",
            }),
            _ => {}
        }
        // Where compaction triggers, which is the line that actually runs out:
        // the window is not the budget.
        if let Some(fill) = session.context.filter(|f| f.compact_at > 0) {
            llm.push(Gauge {
                key: "compact_at",
                label: "COMPACTS AT",
                value: fill.compact_at as f64,
                max: Some(fill.total.max(1) as f64),
                unit: "tok",
            });
        }
        if let Some(usage) = session.usage {
            llm.push(Gauge {
                key: "cached",
                label: "CACHED",
                value: usage.cached_prompt as f64,
                max: Some(usage.prompt.max(1) as f64),
                unit: "tok",
            });
        }
        llm.push(Gauge {
            key: "total_prefill",
            label: "TOTAL PREFILL",
            value: session.prefilled as f64,
            max: None,
            unit: "tok",
        });
        llm.push(Gauge {
            key: "total_output",
            label: "TOTAL OUTPUT",
            value: session.generated as f64,
            max: None,
            unit: "tok",
        });
        llm.push(Gauge {
            key: "last_prefill",
            label: "LAST PREFILL",
            value: session.last_prefill as f64,
            max: None,
            unit: "tok",
        });
        llm.push(Gauge {
            key: "last_output",
            label: "LAST OUTPUT",
            value: session.last_generated as f64,
            max: None,
            unit: "tok",
        });
        llm.push(Gauge {
            key: "prefill_rate",
            label: "PREFILL",
            value: session.rates.prefill(),
            max: None,
            unit: "tok/s",
        });
        llm.push(Gauge {
            key: "decode_rate",
            label: "DECODE",
            value: session.rates.decode(),
            max: None,
            unit: "tok/s",
        });
        llm.push(Gauge {
            key: "requests",
            label: "REQUESTS",
            value: session.requests as f64,
            max: None,
            unit: "",
        });

        for gauge in gauges.iter().chain(llm.iter()) {
            let series = self.history.entry(gauge.key).or_default();
            // Unbounded readings graph as a rate rather than a total, or the
            // line only ever goes up and says nothing.
            let point = match gauge.fraction() {
                Some(fraction) => fraction,
                None => {
                    let last = series.back().copied().unwrap_or(0.0);
                    let _ = last;
                    0.0
                }
            };
            series.push_back(point);
            while series.len() > HISTORY {
                series.pop_front();
            }
            series.make_contiguous();
        }
        self.hardware = gauges;
        self.llm = llm;
    }
}

/// The first amdgpu render device that exposes a busy percentage.
fn find_gpu() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    let mut cards: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("card") && !name.contains('-'))
        })
        .collect();
    cards.sort();
    cards
        .into_iter()
        .map(|card| card.join("device"))
        .find(|device| device.join("gpu_busy_percent").exists())
}

fn read_text(path: impl AsRef<std::path::Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn read_number(path: &std::path::Path) -> Option<f64> {
    read_text(path)?.trim().parse().ok()
}

/// Total and idle jiffies from the aggregate `cpu` line.
fn parse_cpu(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|n| n.parse().ok())
        .collect();
    if fields.len() < 5 {
        return None;
    }
    // idle + iowait, which is what every other tool counts as not-working.
    let idle = fields[3] + fields[4];
    Some((fields.iter().sum(), idle))
}

/// Total and available memory, in kibibytes.
///
/// `MemAvailable` rather than `MemFree`: free memory excludes the page cache,
/// so a healthy machine reads as nearly full and the bar is always at 95%.
fn parse_meminfo(meminfo: &str) -> Option<(f64, f64)> {
    let field = |name: &str| {
        meminfo
            .lines()
            .find(|line| line.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<f64>()
            .ok()
    };
    Some((field("MemTotal:")?, field("MemAvailable:")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percentage_is_over_the_interval_not_since_boot() {
        let first = "cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 2 3 4\n";
        let (total, idle) = parse_cpu(first).unwrap();
        assert_eq!((total, idle), (1000, 800));

        // Half the next interval was idle.
        let second = "cpu  150 0 150 900 0 0 0 0 0 0\n";
        let (total2, idle2) = parse_cpu(second).unwrap();
        let busy = 1.0 - (idle2 - idle) as f64 / (total2 - total) as f64;
        assert!((busy - 0.5).abs() < 0.001, "{busy}");
    }

    #[test]
    fn a_stat_file_without_the_aggregate_line_reads_as_nothing() {
        assert_eq!(parse_cpu("cpu0 1 2 3 4 5\n"), None);
        assert_eq!(parse_cpu(""), None);
        assert_eq!(parse_cpu("cpu  1 2\n"), None, "too few fields");
    }

    /// `MemFree` excludes the page cache, so a healthy machine reads as nearly
    /// full and the bar sits at 95% forever. `MemAvailable` is the honest one.
    #[test]
    fn memory_uses_available_rather_than_free() {
        let meminfo = "\
MemTotal:       32000000 kB
MemFree:          500000 kB
MemAvailable:   20000000 kB
Buffers:          100000 kB
";
        let (total, available) = parse_meminfo(meminfo).unwrap();
        assert_eq!(total, 32_000_000.0);
        assert_eq!(available, 20_000_000.0);
        let used_fraction = (total - available) / total;
        assert!((used_fraction - 0.375).abs() < 0.001, "{used_fraction}");
    }

    #[test]
    fn a_meminfo_without_the_fields_reads_as_nothing() {
        assert_eq!(parse_meminfo("Something: 1 kB\n"), None);
        assert_eq!(parse_meminfo(""), None);
    }

    /// A bar needs a maximum. A reading without one is a graph and nothing
    /// else, which is exactly the radeontop and btop split.
    #[test]
    fn only_bounded_gauges_have_a_bar() {
        let bounded = Gauge {
            key: "gpu",
            label: "GPU",
            value: 32.5,
            max: Some(100.0),
            unit: "%",
        };
        assert_eq!(bounded.fraction(), Some(0.325));
        let unbounded = Gauge {
            key: "prefill",
            label: "PREFILL",
            value: 4093.0,
            max: None,
            unit: "tok",
        };
        assert_eq!(unbounded.fraction(), None);
        // A zero maximum is not a bar either, and must not divide by zero.
        let empty = Gauge {
            max: Some(0.0),
            ..bounded.clone()
        };
        assert_eq!(empty.fraction(), None);
    }

    /// A reading over its maximum clamps rather than drawing past the track.
    #[test]
    fn a_reading_past_its_maximum_is_clamped() {
        let over = Gauge {
            key: "gpu",
            label: "GPU",
            value: 140.0,
            max: Some(100.0),
            unit: "%",
        };
        assert_eq!(over.fraction(), Some(1.0));
    }

    #[test]
    fn a_reading_is_written_the_way_its_unit_wants() {
        let percent = Gauge {
            key: "gpu",
            label: "GPU",
            value: 32.5,
            max: Some(100.0),
            unit: "%",
        };
        assert_eq!(percent.reading(), "32%");
        let memory = Gauge {
            key: "vram",
            label: "VRAM",
            value: 1619.0,
            max: Some(1875.0),
            unit: "MiB",
        };
        assert_eq!(memory.reading(), "1619 / 1875 MiB");
        let tokens = Gauge {
            key: "total_prefill",
            label: "TOTAL PREFILL",
            value: 4093.0,
            max: None,
            unit: "tok",
        };
        assert_eq!(tokens.reading(), "4093");
        // A rate nobody has measured yet says so rather than claiming zero.
        let unmeasured = Gauge {
            key: "decode_rate",
            label: "DECODE",
            value: 0.0,
            max: None,
            unit: "tok/s",
        };
        assert_eq!(unmeasured.reading(), "—");
        let measured = Gauge {
            value: 47.25,
            ..unmeasured.clone()
        };
        assert_eq!(measured.reading(), "47.2 tok/s");
    }

    /// A machine with no amdgpu must produce a monitor, not an error.
    #[test]
    fn a_machine_without_a_gpu_still_reports_what_it_has() {
        let mut monitor = Monitor {
            gpu: None,
            cpu_prev: None,
            hardware: Vec::new(),
            llm: Vec::new(),
            history: HashMap::new(),
        };
        let state = crate::state::State::new();
        monitor.sample(&state);
        monitor.sample(&state);
        let hardware: Vec<&str> = monitor.hardware().iter().map(|g| g.key).collect();
        let llm: Vec<&str> = monitor.llm().iter().map(|g| g.key).collect();
        assert!(llm.contains(&"total_prefill"), "{llm:?}");
        assert!(!hardware.contains(&"gpu"), "{hardware:?}");
        // The two lists are two questions and must not share a row.
        assert!(
            hardware.iter().all(|key| !llm.contains(key)),
            "{hardware:?} and {llm:?} overlap"
        );
        // The real machine's readings appear when they exist; this only
        // asserts that their absence is survivable.
    }

    /// History is bounded, or a window left open overnight is a memory leak
    /// with a graph on it.
    #[test]
    fn history_is_bounded_and_oldest_first() {
        let mut monitor = Monitor {
            gpu: None,
            cpu_prev: None,
            hardware: Vec::new(),
            llm: Vec::new(),
            history: HashMap::new(),
        };
        let mut state = crate::state::State::new();
        for n in 0..HISTORY + 50 {
            state.apply(noob_proto::Event::UsageReport {
                usage: noob_proto::Usage {
                    prompt: n as u64 * 10,
                    cached_prompt: 0,
                    completion: 1,
                    context_total: 10_000,
                },
            });
            monitor.sample(&state);
        }
        let series = monitor.history("context");
        assert_eq!(series.len(), HISTORY);
        assert!(
            series.first() < series.last(),
            "oldest first: {:?} then {:?}",
            series.first(),
            series.last()
        );
    }

    #[test]
    fn asking_for_a_series_nobody_recorded_is_empty_not_a_panic() {
        let monitor = Monitor {
            gpu: None,
            cpu_prev: None,
            hardware: Vec::new(),
            llm: Vec::new(),
            history: HashMap::new(),
        };
        assert!(monitor.history("nothing").is_empty());
    }
}
