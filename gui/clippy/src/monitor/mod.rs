//! What the machine and this run are costing.
//!
//! Three lists, because they answer three questions. HARDWARE is whether the
//! machine is keeping up, out of `/sys` and `/proc`. CONTEXT is what this run is
//! holding right now and what its last request cost. SESSION is what this run
//! has spent altogether and how fast it moved. The last two both come out of the
//! event stream, so every number in this module is the window that is open.
//!
//! Nothing here reads anything that outlives the window. There was a pane of
//! all-time counts, and then a settings block of them: a column of numbers from
//! sessions nobody remembers reads as this session's however it is labelled,
//! and both are gone along with the file behind them.
//!
//! All three are the same [`Gauge`]: a `max` means the value is a proportion and
//! is drawn as a block of dots, and without one the reading is the number alone.
//!
//! The counts the CONTEXT pane heads itself with are not here. Requests, tool
//! calls and failed calls are events rather than samples, so they live on
//! [`crate::state::State`] where the events land, and the pane reads them
//! straight off it.
//!
//! Everything the machine reports is read from `/sys` and `/proc` as text. No
//! dependency, no vendor library, and nothing that fails harder than reporting
//! one fewer reading: a machine without amdgpu simply has no GPU rows.
//!
//! Sampling only runs while this view is on screen. A monitor is inherently
//! periodic and periodic is the opposite of the redraw-on-change rule, so the
//! rule is kept by not sampling when nobody is looking.

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

/// Which slot of the gauge palette each metric wears, by name so a reading is
/// read rather than counted. Two readings in one pane must never share one, and
/// a metric that appears in two panes keeps the same one: prefill is blue in
/// both, so LAST PREFILL in the context pane and PREFILLED in the session pane
/// read as the same quantity measured twice.
const HUE_RED: usize = 0;
const HUE_ORANGE: usize = 1;
const HUE_YELLOW: usize = 2;
const HUE_LIME: usize = 3;
const HUE_GREEN: usize = 4;
const HUE_TEAL: usize = 5;
const HUE_BLUE: usize = 6;
const HUE_INDIGO: usize = 7;
// Slots 8 and 9 have no name because no reading wears them. The palette in
// `skin.rs` is ten wide and wraps, so an unnamed slot costs nothing until
// something claims it. Slot 8 was TOTAL TOOL CALLS, which is a header row in
// the CONTEXT pane now rather than a reading with a colour.

pub struct Monitor {
    /// The amdgpu device directory, when there is one.
    gpu: Option<PathBuf>,
    /// Total and idle jiffies from the previous `/proc/stat` read, so the
    /// percentage is over the interval rather than since boot.
    cpu_prev: Option<(u64, u64)>,
    hardware: Vec<Gauge>,
    context: Vec<Gauge>,
    session: Vec<Gauge>,
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
            context: Vec::new(),
            session: Vec::new(),
        }
    }

    /// What the machine is doing.
    pub fn hardware(&self) -> Vec<Gauge> {
        self.hardware.clone()
    }

    /// What the CONTEXT pane draws as readings: how full the window is, and
    /// what the last response cost. The counts above them are header rows the
    /// pane takes off the state itself. Named for the pane it feeds, because it
    /// was called `session` while feeding CONTEXT and that is a trap for
    /// whoever reads it next.
    pub fn context(&self) -> Vec<Gauge> {
        self.context.clone()
    }

    /// What the SESSION pane shows: the tokens this run has moved and the rates
    /// it moved them at. Separate from the context because they are different
    /// questions, one is how full the window is and the other is what filling it
    /// cost.
    pub fn session(&self) -> Vec<Gauge> {
        self.session.clone()
    }

    /// Read every source once. Cheap: six small files, no allocation past the
    /// strings they contain.
    ///
    /// The state is the only argument. It used to take an all-time totals file
    /// with this run added on top, for a pane that is gone: both token lists are
    /// this run and nothing else.
    pub fn sample(&mut self, state: &crate::state::State) {
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

        // How full the window is, and what it took to fill it. The one bounded
        // reading in this pane is the fill itself, which is the reading the pane
        // is named after.
        //
        // The agent's own reading where it sent one: it moves at every
        // transcript boundary, while usage only reports the request that
        // already went out. Falling back keeps a stream without measurements
        // showing something true rather than nothing.
        let mut context = Vec::new();
        match (state.context, state.usage) {
            (Some(fill), _) if fill.total > 0 => context.push(Gauge {
                key: "context",
                label: "CONTEXT",
                value: fill.used as f64,
                max: Some(fill.total as f64),
                unit: "tok",
                hue: HUE_GREEN,
            }),
            (_, Some(usage)) => context.push(Gauge {
                key: "context",
                label: "CONTEXT",
                value: usage.prompt as f64,
                max: Some(usage.context_total as f64),
                unit: "tok",
                hue: HUE_GREEN,
            }),
            _ => {}
        }
        // The three counts that said how much work went into that fill (total
        // requests, total tool calls, last prefill) are not readings any more:
        // they are labelled rows in the pane's own header, where they read
        // beside the phase instead of under a dot block. What is left here is
        // the fill and the last response, which is the one number the header
        // does not carry.
        context.push(Gauge {
            key: "last_generated",
            label: "LAST GENERATED",
            value: state.last_generated as f64,
            max: None,
            unit: "tok",
            hue: HUE_YELLOW,
        });

        // What this run has spent: the three token counts, then the rates they
        // were moved at. Measured rather than reported, which is what says
        // whether something is wrong right now.
        //
        // Read out of the live state. That is the whole of item 22: the pane
        // used to show a file of past sessions and there was nothing on it to
        // say so.
        let session = vec![
            Gauge {
                key: "prefilled",
                label: "PREFILLED",
                value: state.prefilled as f64,
                max: None,
                unit: "tok",
                hue: HUE_BLUE,
            },
            Gauge {
                key: "generated",
                label: "GENERATED",
                value: state.generated as f64,
                max: None,
                unit: "tok",
                hue: HUE_YELLOW,
            },
            Gauge {
                key: "cached",
                label: "CACHED",
                value: state.cached_prefill as f64,
                max: None,
                unit: "tok",
                hue: HUE_TEAL,
            },
            Gauge {
                key: "prefill_rate",
                label: "PREFILL",
                value: state.rates.prefill(),
                max: None,
                unit: "tok/s",
                hue: HUE_INDIGO,
            },
            Gauge {
                key: "decode_rate",
                label: "DECODE",
                value: state.rates.decode(),
                max: None,
                unit: "tok/s",
                hue: HUE_LIME,
            },
        ];

        // The pane's order is a decision, not the accident of which source
        // was read first. A reading whose key is not on the list (a future
        // gauge) goes after the named ones rather than vanishing.
        gauges.sort_by_key(|gauge| hardware_order(gauge.key));
        self.hardware = gauges;
        self.context = context;
        self.session = session;
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
/// Where a hardware reading stands in the pane: GPU, CPU, GTT, RAM, VRAM.
fn hardware_order(key: &str) -> usize {
    ["gpu", "cpu", "gtt", "ram", "vram"]
        .iter()
        .position(|name| *name == key)
        .unwrap_or(usize::MAX)
}

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
    /// The pane's declared order: GPU, CPU, GTT, RAM, VRAM, and an unknown
    /// key sorts after every named one.
    #[test]
    fn hardware_readings_stand_in_the_declared_order() {
        let mut keys = ["vram", "ram", "gtt", "cpu", "gpu", "later"];
        keys.sort_by_key(|key| super::hardware_order(key));
        assert_eq!(keys, ["gpu", "cpu", "gtt", "ram", "vram", "later"]);
    }

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
            hue: HUE_INDIGO,
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
            context: Vec::new(),
            session: Vec::new(),
        }
    }

    /// A machine with no amdgpu must produce a monitor, not an error.
    #[test]
    fn a_machine_without_a_gpu_still_reports_what_it_has() {
        let mut monitor = bare();
        let state = crate::state::State::new();
        monitor.sample(&state);
        monitor.sample(&state);
        let hardware: Vec<&str> = monitor.hardware().iter().map(|g| g.key).collect();
        let context: Vec<&str> = monitor.context().iter().map(|g| g.key).collect();
        assert!(context.contains(&"last_generated"), "{context:?}");
        assert!(!hardware.contains(&"gpu"), "{hardware:?}");
        // The lists are separate questions and must not share a row.
        let session: Vec<&str> = monitor.session().iter().map(|g| g.key).collect();
        for (a, b) in [(&hardware, &context), (&hardware, &session), (&context, &session)] {
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
    ///
    /// This asserted BEST OUTPUT in the context pane and seven all-time readings
    /// in a third list read out of a totals file. Both are gone: the context
    /// pane carries the fill and what it cost, and the session pane carries this
    /// run's own spend.
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
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 1_500,
                cached_prompt: 900,
                completion: 20,
                context_total: 65_536,
            },
        });
        monitor.sample(&state);

        let context: Vec<&str> = monitor.context().iter().map(|g| g.key).collect();
        // Two readings, not five. TOTAL REQUESTS, TOTAL TOOL CALLS and LAST
        // PREFILL are header rows of the pane now, read straight off the state,
        // and a gauge for any of them would draw the same number twice.
        assert_eq!(context, vec!["context", "last_generated"]);
        let session: Vec<&str> = monitor.session().iter().map(|g| g.key).collect();
        assert_eq!(
            session,
            vec!["prefilled", "generated", "cached", "prefill_rate", "decode_rate"]
        );

        let read = |gauges: Vec<Gauge>, key: &str| {
            gauges
                .into_iter()
                .find(|g| g.key == key)
                .unwrap_or_else(|| panic!("{key}"))
        };
        // The context pane: how full the window is, and what the last response
        // cost. The counts its header carries are the state's own, which is
        // where the pane reads them.
        assert_eq!(read(monitor.context(), "context").value, 1_500.0);
        assert_eq!(read(monitor.context(), "last_generated").value, 20.0);
        assert_eq!(state.requests, 2);
        assert_eq!(state.tool_calls, 1);
        assert_eq!(state.last_prefill, 600);
        // The session pane: read out of the live run and nothing else.
        assert_eq!(read(monitor.session(), "prefilled").value, 1_100.0);
        assert_eq!(read(monitor.session(), "generated").value, 80.0);
        assert_eq!(read(monitor.session(), "cached").value, 1_300.0);
        assert_eq!(
            read(monitor.session(), "prefilled").value,
            state.prefilled as f64,
            "the pane and the state have to agree"
        );
    }

    /// A pane that reads a file is the bug item 22 reported: OVERALL showed
    /// prefilled and generated from somewhere unexplained. Every reading here is
    /// this run, so sampling the same run twice cannot move one.
    ///
    /// This asserted the same thing against a live `crate::totals::Totals`,
    /// which no longer exists: the all-time file went with the settings section
    /// that was the last thing reading it. What it asserts now is the property
    /// that survived, which is that the panes are the state and nothing else.
    #[test]
    fn no_pane_reads_anything_but_this_run() {
        let mut monitor = bare();
        let mut state = crate::state::State::new();
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 900,
                cached_prompt: 400,
                completion: 60,
                context_total: 65_536,
            },
        });
        monitor.sample(&state);
        // The token lists only: the hardware ones are /sys and are supposed to
        // move between two samples.
        let reading = |monitor: &Monitor| -> Vec<Gauge> {
            monitor.context().into_iter().chain(monitor.session()).collect()
        };
        let before = reading(&monitor);
        monitor.sample(&state);
        let after = reading(&monitor);
        assert_eq!(before, after, "a second sample of the same run moved");
        // Nothing carries a number this run never produced: one request of 900
        // prompt tokens and 60 completion bounds every count on these two panes.
        for gauge in &after {
            assert!(
                gauge.value <= 65_536.0,
                "{} reads more than this run did: {}",
                gauge.key,
                gauge.value
            );
        }
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
        monitor.sample(&state);
        let slots = crate::skin::Skin::default().gauges.len();
        for pane in [monitor.hardware(), monitor.context(), monitor.session()] {
            let mut seen = Vec::new();
            for gauge in &pane {
                assert!(gauge.hue < slots, "{} names slot {}", gauge.key, gauge.hue);
                assert!(!seen.contains(&gauge.hue), "{} shares a hue", gauge.key);
                seen.push(gauge.hue);
            }
        }
    }

    /// Sampling forever must not grow anything. Every list is replaced on each
    /// read rather than appended to, so a window left open overnight holds one
    /// sample and not a night of them. There used to be a ring of past readings
    /// behind these, kept for a graph nothing ever drew.
    ///
    /// Only the two token panes are counted. The hardware list is what the
    /// machine reports, and its CPU row is a difference between two reads of
    /// `/proc/stat`, so two samples inside one jiffy legitimately produce no CPU
    /// reading at all.
    #[test]
    fn sampling_forever_holds_one_reading_per_row() {
        let mut monitor = bare();
        let mut state = crate::state::State::new();
        let mut first = Vec::new();
        for n in 0..300 {
            state.apply(noob_proto::Event::UsageReport {
                usage: noob_proto::Usage {
                    prompt: n as u64 * 10,
                    cached_prompt: 0,
                    completion: 1,
                    context_total: 10_000,
                },
            });
            monitor.sample(&state);
            let keys: Vec<&str> = monitor
                .context()
                .iter()
                .chain(monitor.session().iter())
                .map(|gauge| gauge.key)
                .collect();
            if n == 0 {
                first = keys;
            } else {
                assert_eq!(keys, first, "sample {n} carries a different set of rows");
            }
        }
        assert!(!first.is_empty(), "a sampled monitor has readings");
    }
}
