//! What the machine, this run and every run are costing.
//!
//! Three lists, because they answer three questions. HARDWARE is whether the
//! machine is keeping up, out of `/sys` and `/proc`. SESSION is what this run is
//! holding and what it has spent, out of the event stream. OVERALL is every run
//! there has ever been, out of [`crate::totals`], which is the only reading here
//! that outlives the window.
//!
//! All three are the same [`Gauge`]: a `max` means the value is a proportion and
//! is drawn as a block of dots, and without one the reading is the number alone.
//!
//! The failed calls the DEBUG pane shows are not here. They are events rather
//! than samples, so they live on [`crate::state::State`] where the events land.
//!
//! Everything the machine reports is read from `/sys` and `/proc` as text. No
//! dependency, no vendor library, and nothing that fails harder than reporting
//! one fewer reading: a machine without amdgpu simply has no GPU rows.
//!
//! Sampling only runs while this view is on screen. A monitor is inherently
//! periodic and periodic is the opposite of the redraw-on-change rule, so the
//! rule is kept by not sampling when nobody is looking.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;

/// One measured quantity. `max` present means it can be drawn as a block of
/// dots; without one there is nothing to be a proportion of, and the reading is
/// the number alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Gauge {
    pub key: &'static str,
    pub label: &'static str,
    pub value: f64,
    pub max: Option<f64>,
    pub unit: &'static str,
    /// Which slot of the gauge palette this metric wears. Named per metric here
    /// rather than taken from the row's position on screen, so a reading keeps
    /// its colour when the row above it is absent.
    pub hue: usize,
}

impl Gauge {
    /// Where the block fills to, 0.0 to 1.0. Unbounded readings have no block.
    pub fn fraction(&self) -> Option<f32> {
        match self.max {
            Some(max) if max > 0.0 => Some((self.value / max).clamp(0.0, 1.0) as f32),
            _ => None,
        }
    }

    /// The reading, written the way the unit wants it. Token counts are grouped
    /// in thousands: these run to seven figures and an ungrouped 1048576 has to
    /// be counted rather than read.
    pub fn reading(&self) -> String {
        let count = |n: f64| crate::state::thousands(n.max(0.0) as u64);
        match (self.unit, self.max) {
            ("%", _) => format!("{:.0}%", self.value),
            ("MiB", Some(max)) => format!("{} / {} MiB", count(self.value), count(max)),
            ("MiB", None) => format!("{} MiB", count(self.value)),
            ("tok", Some(max)) => format!("{} / {}", count(self.value), count(max)),
            ("tok", None) => count(self.value),
            ("tok/s", _) if self.value <= 0.0 => String::from("—"),
            ("tok/s", _) => format!("{:.1} tok/s", self.value),
            ("", _) => count(self.value),
            (unit, _) => format!("{:.1} {unit}", self.value),
        }
    }
}

const HISTORY: usize = 240;

/// Which slot of the gauge palette each metric wears, by name so a reading is
/// read rather than counted. Two readings in one pane must never share one, and
/// a metric that appears in two panes keeps the same one: PREFILLED is the same
/// blue in the session pane and the overall pane, which is what makes the pair
/// legible side by side.
const HUE_RED: usize = 0;
const HUE_ORANGE: usize = 1;
const HUE_YELLOW: usize = 2;
const HUE_LIME: usize = 3;
const HUE_GREEN: usize = 4;
const HUE_TEAL: usize = 5;
const HUE_BLUE: usize = 6;
const HUE_INDIGO: usize = 7;
const HUE_VIOLET: usize = 8;
const HUE_PINK: usize = 9;

pub struct Monitor {
    /// The amdgpu device directory, when there is one.
    gpu: Option<PathBuf>,
    /// Total and idle jiffies from the previous `/proc/stat` read, so the
    /// percentage is over the interval rather than since boot.
    cpu_prev: Option<(u64, u64)>,
    hardware: Vec<Gauge>,
    session: Vec<Gauge>,
    overall: Vec<Gauge>,
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
            session: Vec::new(),
            overall: Vec::new(),
            history: HashMap::new(),
        }
    }

    /// What the machine is doing.
    pub fn hardware(&self) -> Vec<Gauge> {
        self.hardware.clone()
    }

    /// What this run is doing. Separate from the hardware because they are
    /// different questions: one is whether the machine is keeping up, the other
    /// is whether the budget is.
    pub fn session(&self) -> Vec<Gauge> {
        self.session.clone()
    }

    /// What every run has done, out of the totals file. Separate from the
    /// session because a total that resets when the window closes cannot answer
    /// "is this machine slower than it was last week".
    pub fn overall(&self) -> Vec<Gauge> {
        self.overall.clone()
    }

    /// Everything recorded for one key, oldest first. Empty until sampled.
    ///
    /// Nothing draws it at the moment. A trend used to be drawn behind the dots
    /// of every reading and came off with the bars: the reference asked for a
    /// block and a number, and a rolling graph behind them was the noise that
    /// made the pane unreadable. The samples are still recorded because the
    /// hardware pane is getting a proper graph of its own, and a graph cannot be
    /// backfilled from sums.
    #[allow(dead_code)]
    pub fn history(&self, key: &str) -> &[f32] {
        self.history
            .get(key)
            .map(|q| q.as_slices().0)
            .unwrap_or(&[])
    }

    /// Read every source once. Cheap: six small files, no allocation past the
    /// strings they contain.
    ///
    /// `overall` is the totals file with this run already added, which the
    /// window does before it calls here: adding it in two places is how the
    /// same session gets counted twice.
    pub fn sample(&mut self, state: &crate::state::State, overall: &crate::totals::Totals) {
        let mut gauges = Vec::new();

        if let Some(gpu) = &self.gpu {
            if let Some(busy) = read_number(&gpu.join("gpu_busy_percent")) {
                gauges.push(Gauge {
                    key: "gpu",
                    label: "GPU",
                    value: busy,
                    max: Some(100.0),
                    unit: "%",
                    hue: HUE_RED,
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
                    hue: HUE_ORANGE,
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
                    hue: HUE_YELLOW,
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
                        hue: HUE_TEAL,
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
                hue: HUE_BLUE,
            });
        }

        // This run's economy, which is a different question from whether the
        // machine is keeping up: this is the budget that runs out first.
        //
        // The agent's own reading where it sent one: it moves at every
        // transcript boundary, while usage only reports the request that
        // already went out. Falling back keeps a stream without measurements
        // showing something true rather than nothing.
        let mut session = Vec::new();
        match (state.context, state.usage) {
            (Some(fill), _) if fill.total > 0 => session.push(Gauge {
                key: "context",
                label: "CONTEXT",
                value: fill.used as f64,
                max: Some(fill.total as f64),
                unit: "tok",
                hue: HUE_GREEN,
            }),
            (_, Some(usage)) => session.push(Gauge {
                key: "context",
                label: "CONTEXT",
                value: usage.prompt as f64,
                max: Some(usage.context_total as f64),
                unit: "tok",
                hue: HUE_GREEN,
            }),
            _ => {}
        }
        // Where compaction triggers, which is the line that actually runs out:
        // the window is not the budget.
        if let Some(fill) = state.context.filter(|f| f.compact_at > 0) {
            session.push(Gauge {
                key: "compact_at",
                label: "COMPACTS AT",
                value: fill.compact_at as f64,
                max: Some(fill.total.max(1) as f64),
                unit: "tok",
                hue: HUE_RED,
            });
        }
        session.push(Gauge {
            key: "tool_calls",
            label: "TOOL CALLS",
            value: state.tool_calls as f64,
            max: None,
            unit: "",
            hue: HUE_VIOLET,
        });
        // The longest single answer, which a total cannot tell you: forty
        // requests of fifty tokens and one of four thousand sum the same as a
        // steady thousand each.
        session.push(Gauge {
            key: "best_output",
            label: "BEST OUTPUT",
            value: state.best_generated as f64,
            max: None,
            unit: "tok",
            hue: HUE_PINK,
        });
        session.push(Gauge {
            key: "prefilled",
            label: "PREFILLED",
            value: state.prefilled as f64,
            max: None,
            unit: "tok",
            hue: HUE_BLUE,
        });
        session.push(Gauge {
            key: "cached",
            label: "CACHED",
            value: state.cached_prefill as f64,
            max: None,
            unit: "tok",
            hue: HUE_TEAL,
        });
        session.push(Gauge {
            key: "generated",
            label: "GENERATED",
            value: state.generated as f64,
            max: None,
            unit: "tok",
            hue: HUE_YELLOW,
        });
        session.push(Gauge {
            key: "requests",
            label: "REQUESTS",
            value: state.requests as f64,
            max: None,
            unit: "",
            hue: HUE_ORANGE,
        });
        // Measured, not reported: how fast this run has actually been going,
        // which is the reading that says whether something is wrong right now.
        // The all-time averages next door move too slowly to tell.
        session.push(Gauge {
            key: "prefill_rate",
            label: "PREFILL",
            value: state.rates.prefill(),
            max: None,
            unit: "tok/s",
            hue: HUE_INDIGO,
        });
        session.push(Gauge {
            key: "decode_rate",
            label: "DECODE",
            value: state.rates.decode(),
            max: None,
            unit: "tok/s",
            hue: HUE_LIME,
        });

        // Every run there has ever been. The same three counts as the session
        // above, plus what a request typically costs: an average over every
        // request ever, and the median beside it, because one cold start with a
        // full transcript drags an average down for the rest of the day.
        let overall = vec![
            Gauge {
                key: "all_prefilled",
                label: "PREFILLED",
                value: overall.prefilled as f64,
                max: None,
                unit: "tok",
                hue: HUE_BLUE,
            },
            Gauge {
                key: "all_generated",
                label: "GENERATED",
                value: overall.generated as f64,
                max: None,
                unit: "tok",
                hue: HUE_YELLOW,
            },
            Gauge {
                key: "all_cached",
                label: "CACHED",
                value: overall.cached as f64,
                max: None,
                unit: "tok",
                hue: HUE_TEAL,
            },
            Gauge {
                key: "decode_mean",
                label: "DECODE MEAN",
                value: overall.average_decode(),
                max: None,
                unit: "tok/s",
                hue: HUE_GREEN,
            },
            Gauge {
                key: "decode_median",
                label: "DECODE MID",
                value: overall.median_decode(),
                max: None,
                unit: "tok/s",
                hue: HUE_LIME,
            },
            Gauge {
                key: "prefill_mean",
                label: "PREFILL MEAN",
                value: overall.average_prefill(),
                max: None,
                unit: "tok/s",
                hue: HUE_ORANGE,
            },
            Gauge {
                key: "prefill_median",
                label: "PREFILL MID",
                value: overall.median_prefill(),
                max: None,
                unit: "tok/s",
                hue: HUE_RED,
            },
        ];

        // Bounded readings only. An unbounded one has no proportion to plot, and
        // recording a flat zero for it was noise in the trend of everything
        // else.
        for gauge in gauges.iter().chain(session.iter()) {
            let Some(fraction) = gauge.fraction() else {
                continue;
            };
            let series = self.history.entry(gauge.key).or_default();
            series.push_back(fraction);
            while series.len() > HISTORY {
                series.pop_front();
            }
            series.make_contiguous();
        }
        self.hardware = gauges;
        self.session = session;
        self.overall = overall;
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

    /// A block needs a maximum. A reading without one is a number and nothing
    /// else, which is exactly the radeontop and btop split.
    #[test]
    fn only_bounded_gauges_have_a_block() {
        let bounded = Gauge {
            key: "gpu",
            label: "GPU",
            value: 32.5,
            max: Some(100.0),
            unit: "%",
            hue: HUE_RED,
        };
        assert_eq!(bounded.fraction(), Some(0.325));
        let unbounded = Gauge {
            key: "prefilled",
            label: "PREFILLED",
            value: 4093.0,
            max: None,
            unit: "tok",
            hue: HUE_BLUE,
        };
        assert_eq!(unbounded.fraction(), None);
        // A zero maximum is not a block either, and must not divide by zero.
        let empty = Gauge {
            max: Some(0.0),
            ..bounded.clone()
        };
        assert_eq!(empty.fraction(), None);
    }

    /// A reading over its maximum clamps rather than drawing past the block.
    #[test]
    fn a_reading_past_its_maximum_is_clamped() {
        let over = Gauge {
            key: "gpu",
            label: "GPU",
            value: 140.0,
            max: Some(100.0),
            unit: "%",
            hue: HUE_RED,
        };
        assert_eq!(over.fraction(), Some(1.0));
    }

    /// Token counts are grouped. This asserted the ungrouped form until the
    /// readings became the whole of what a monitor row shows: a seven figure
    /// prefill total written 1048576 has to be counted rather than read.
    #[test]
    fn a_reading_is_written_the_way_its_unit_wants() {
        let percent = Gauge {
            key: "gpu",
            label: "GPU",
            value: 32.5,
            max: Some(100.0),
            unit: "%",
            hue: HUE_RED,
        };
        assert_eq!(percent.reading(), "32%");
        let memory = Gauge {
            key: "vram",
            label: "VRAM",
            value: 1619.0,
            max: Some(1875.0),
            unit: "MiB",
            hue: HUE_ORANGE,
        };
        assert_eq!(memory.reading(), "1,619 / 1,875 MiB");
        let tokens = Gauge {
            key: "prefilled",
            label: "PREFILLED",
            value: 1_048_576.0,
            max: None,
            unit: "tok",
            hue: HUE_BLUE,
        };
        assert_eq!(tokens.reading(), "1,048,576");
        let count = Gauge {
            key: "tool_calls",
            label: "TOOL CALLS",
            value: 1234.0,
            max: None,
            unit: "",
            hue: HUE_VIOLET,
        };
        assert_eq!(count.reading(), "1,234");
        // A rate nobody has measured yet says so rather than claiming zero.
        let unmeasured = Gauge {
            key: "decode_mean",
            label: "DECODE MEAN",
            value: 0.0,
            max: None,
            unit: "tok/s",
            hue: HUE_GREEN,
        };
        assert_eq!(unmeasured.reading(), "\u{2014}");
        let measured = Gauge {
            value: 47.25,
            ..unmeasured.clone()
        };
        assert_eq!(measured.reading(), "47.2 tok/s");
    }

    fn bare() -> Monitor {
        Monitor {
            gpu: None,
            cpu_prev: None,
            hardware: Vec::new(),
            session: Vec::new(),
            overall: Vec::new(),
            history: HashMap::new(),
        }
    }

    /// A machine with no amdgpu must produce a monitor, not an error.
    #[test]
    fn a_machine_without_a_gpu_still_reports_what_it_has() {
        let mut monitor = bare();
        let state = crate::state::State::new();
        let totals = crate::totals::Totals::default();
        monitor.sample(&state, &totals);
        monitor.sample(&state, &totals);
        let hardware: Vec<&str> = monitor.hardware().iter().map(|g| g.key).collect();
        let session: Vec<&str> = monitor.session().iter().map(|g| g.key).collect();
        assert!(session.contains(&"tool_calls"), "{session:?}");
        assert!(!hardware.contains(&"gpu"), "{hardware:?}");
        // The lists are separate questions and must not share a row.
        let overall: Vec<&str> = monitor.overall().iter().map(|g| g.key).collect();
        for (a, b) in [(&hardware, &session), (&hardware, &overall), (&session, &overall)] {
            assert!(
                a.iter().all(|key| !b.contains(key)),
                "{a:?} and {b:?} overlap"
            );
        }
        // The real machine's readings appear when they exist; this only
        // asserts that their absence is survivable.
    }

    /// The three lists are the three questions that were asked for, by name.
    /// A reading in the wrong pane is the complaint this split came from.
    #[test]
    fn each_pane_carries_the_readings_it_was_asked_for() {
        let mut monitor = bare();
        let mut state = crate::state::State::new();
        state.apply(noob_proto::Event::ToolStart {
            call_id: "c1".into(),
            name: "bash".into(),
            brief: "ls".into(),
            args: serde_json::json!({}),
        });
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 900,
                cached_prompt: 400,
                completion: 60,
                context_total: 65_536,
            },
        });
        let totals = crate::totals::Totals {
            prefilled: 5_000,
            generated: 900,
            cached: 4_000,
            decode_tokens: 900,
            decode_seconds: 30.0,
            decode_rates: vec![30.0, 31.0],
            prefill_tokens: 5_000,
            prefill_seconds: 2.0,
            prefill_rates: vec![2500.0],
        };
        monitor.sample(&state, &totals);

        let session: Vec<&str> = monitor.session().iter().map(|g| g.key).collect();
        for wanted in ["context", "tool_calls", "best_output"] {
            assert!(session.contains(&wanted), "{wanted} is not in {session:?}");
        }
        let overall: Vec<&str> = monitor.overall().iter().map(|g| g.key).collect();
        for wanted in [
            "all_prefilled",
            "all_generated",
            "all_cached",
            "decode_mean",
            "decode_median",
            "prefill_mean",
            "prefill_median",
        ] {
            assert!(overall.contains(&wanted), "{wanted} is not in {overall:?}");
        }

        let read = |gauges: Vec<Gauge>, key: &str| {
            gauges
                .into_iter()
                .find(|g| g.key == key)
                .unwrap_or_else(|| panic!("{key}"))
        };
        // This run only, out of the events.
        assert_eq!(read(monitor.session(), "tool_calls").value, 1.0);
        assert_eq!(read(monitor.session(), "best_output").value, 60.0);
        assert_eq!(read(monitor.session(), "cached").value, 400.0);
        // Every run, out of the file, which already has this one added.
        assert_eq!(read(monitor.overall(), "all_prefilled").value, 5_000.0);
        assert_eq!(read(monitor.overall(), "all_cached").value, 4_000.0);
        assert_eq!(read(monitor.overall(), "decode_mean").value, 30.0);
        assert_eq!(read(monitor.overall(), "decode_median").value, 30.5);
    }

    /// The colour is what says which block belongs to which label, so no two
    /// readings in one pane may share one, and every one of them has to name a
    /// slot the palette actually has.
    #[test]
    fn every_reading_in_a_pane_has_its_own_hue() {
        let mut monitor = bare();
        let mut state = crate::state::State::new();
        state.apply(noob_proto::Event::Metrics {
            group: "context".into(),
            at_ms: 0,
            samples: vec![
                noob_proto::Sample {
                    key: "used".into(),
                    label: "used".into(),
                    value: 2_000.0,
                    max: Some(65_536.0),
                    unit: None,
                },
                noob_proto::Sample {
                    key: "compact_at".into(),
                    label: "compacts at".into(),
                    value: 50_000.0,
                    max: None,
                    unit: None,
                },
            ],
        });
        monitor.sample(&state, &crate::totals::Totals::default());
        let slots = crate::skin::Skin::default().gauges.len();
        for pane in [monitor.hardware(), monitor.session(), monitor.overall()] {
            let mut seen = Vec::new();
            for gauge in &pane {
                assert!(gauge.hue < slots, "{} names slot {}", gauge.key, gauge.hue);
                assert!(!seen.contains(&gauge.hue), "{} shares a hue", gauge.key);
                seen.push(gauge.hue);
            }
        }
    }

    /// History is bounded, or a window left open overnight is a memory leak
    /// with a graph on it.
    #[test]
    fn history_is_bounded_and_oldest_first() {
        let mut monitor = bare();
        let mut state = crate::state::State::new();
        let totals = crate::totals::Totals::default();
        for n in 0..HISTORY + 50 {
            state.apply(noob_proto::Event::UsageReport {
                usage: noob_proto::Usage {
                    prompt: n as u64 * 10,
                    cached_prompt: 0,
                    completion: 1,
                    context_total: 10_000,
                },
            });
            monitor.sample(&state, &totals);
        }
        let series = monitor.history("context");
        assert_eq!(series.len(), HISTORY);
        assert!(
            series.first() < series.last(),
            "oldest first: {:?} then {:?}",
            series.first(),
            series.last()
        );
        // An unbounded reading has no proportion to record, so it has no series
        // rather than a flat line of zeroes.
        assert!(monitor.history("prefilled").is_empty());
    }

    #[test]
    fn asking_for_a_series_nobody_recorded_is_empty_not_a_panic() {
        assert!(bare().history("nothing").is_empty());
    }
}
