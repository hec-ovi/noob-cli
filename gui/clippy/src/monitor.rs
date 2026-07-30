//! What the machine and this run are costing.
//!
//! Three lists, because they answer three questions. HARDWARE is whether the
//! machine is keeping up, out of `/sys` and `/proc`. CONTEXT is what this run is
//! holding right now and what its last request cost. SESSION is what this run
//! has spent altogether and how fast it moved. The last two both come out of the
//! event stream, so every number in this module is the window that is open.
//!
//! Nothing here reads the all-time totals any more. [`crate::totals`] is still
//! recorded and still written at the end of every turn, it simply has no pane:
//! a column of counts from sessions nobody remembers was read as this session's,
//! which is the confusion it came off for. Those numbers belong in the settings
//! panel, as a block that says what it is.
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
const HUE_VIOLET: usize = 8;
// Slot 9 has no name because no reading wears it. The palette in `skin.rs` is
// ten wide and wraps, so an unnamed slot costs nothing until something claims
// it.

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

    /// What the CONTEXT pane shows: how full the window is, how many requests
    /// and calls it took to get there, and what the last request alone cost.
    /// Named for the pane it feeds, because it was called `session` while
    /// feeding CONTEXT and that is a trap for whoever reads it next.
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
    /// The state is the only argument. It used to take the totals file with this
    /// run added on top, for a pane that is gone: both token lists are this run
    /// and nothing else, so there is nothing here to confuse with the numbers in
    /// [`crate::totals`].
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
        // How much work went into that fill: every request and every call this
        // run has made, which is why TOTAL is in the label. The two beneath them
        // are the last request alone, so a pane of totals still says what one
        // request currently costs.
        context.push(Gauge {
            key: "requests",
            label: "TOTAL REQUESTS",
            value: state.requests as f64,
            max: None,
            unit: "",
            hue: HUE_ORANGE,
        });
        context.push(Gauge {
            key: "tool_calls",
            label: "TOTAL TOOL CALLS",
            value: state.tool_calls as f64,
            max: None,
            unit: "",
            hue: HUE_VIOLET,
        });
        context.push(Gauge {
            key: "last_prefill",
            label: "LAST PREFILL",
            value: state.last_prefill as f64,
            max: None,
            unit: "tok",
            hue: HUE_BLUE,
        });
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
        // These read the same numbers the totals file is written from, out of
        // the live state instead of the file. That is the whole of item 22: the
        // pane used to show the file and there was nothing on it to say so.
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
        assert!(context.contains(&"tool_calls"), "{context:?}");
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
    /// in a third list read out of the totals file. Both are gone: the context
    /// pane carries the fill and what it cost, the session pane carries this
    /// run's own spend, and the file has no pane at all until the settings panel
    /// gets one.
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
        assert_eq!(
            context,
            vec![
                "context",
                "requests",
                "tool_calls",
                "last_prefill",
                "last_generated"
            ]
        );
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
        // The context pane: this run's totals of work done, and the last request
        // on its own beneath them.
        assert_eq!(read(monitor.context(), "requests").value, 2.0);
        assert_eq!(read(monitor.context(), "tool_calls").value, 1.0);
        assert_eq!(read(monitor.context(), "last_prefill").value, 600.0);
        assert_eq!(read(monitor.context(), "last_generated").value, 20.0);
        // The session pane: the same numbers the totals file is written from,
        // read out of the live run rather than out of the file.
        assert_eq!(read(monitor.session(), "prefilled").value, 1_100.0);
        assert_eq!(read(monitor.session(), "generated").value, 80.0);
        assert_eq!(read(monitor.session(), "cached").value, 1_300.0);
        assert_eq!(
            read(monitor.session(), "prefilled").value,
            state.prefilled as f64,
            "the pane and the state have to agree"
        );
    }

    /// A pane that reads the file is the bug item 22 reported: OVERALL showed
    /// prefilled and generated from somewhere unexplained. Nothing in the
    /// monitor may move when the totals file does.
    #[test]
    fn no_pane_reads_the_all_time_totals() {
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
        // A file with millions in it, still written and still loaded, and the
        // panes do not know it exists.
        let file = crate::totals::Totals {
            prefilled: 4_200_000,
            generated: 90_000,
            cached: 3_100_000,
            ..crate::totals::Totals::default()
        };
        assert_eq!(file.plus(&state).prefilled, 4_200_500, "the file still adds");
        monitor.sample(&state);
        let after = reading(&monitor);
        assert_eq!(before, after, "a second sample of the same run moved");
        for gauge in &after {
            assert!(
                gauge.value < 4_200_000.0,
                "{} is reading the file: {}",
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
