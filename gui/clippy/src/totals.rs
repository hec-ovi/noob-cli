//! The readings that outlive one window: tokens moved, and how fast.
//!
//! Everything else in the window is this session and dies with it. These are
//! carried in a small file beside the settings, in the same `key = value` shape,
//! so the monitor reading them can answer "how much has this machine done"
//! rather than "how much since you opened the window".
//!
//! The file holds the sessions that came before, never the one running. The
//! window keeps this loaded copy untouched and adds the live session to it with
//! [`Totals::plus`] whenever it needs a reading or writes the file, so writing
//! twice in one session cannot count that session twice.
//!
//! Tolerant on the way in, atomic on the way out. A missing file is a first run,
//! a corrupt line is a line that does not exist, and a value that cannot be a
//! number keeps the default: a totals file is a nicety, and refusing to open the
//! window over one would be absurd. The write goes through the same
//! rename-into-place path the settings writer uses, so a crash mid-write cannot
//! leave half a file behind.

use std::path::{Path, PathBuf};

/// How many per-request rate samples are kept, per phase.
///
/// A median needs the samples themselves: a running sum gives an average and
/// has already forgotten which requests it was made of. Bounded because a file
/// that grew one line per request forever would be a session log, and the
/// newest samples are the ones that describe the machine as it is now.
pub const SAMPLES: usize = 512;

/// Every number the file carries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Totals {
    /// Tokens the endpoint actually had to compute, all time.
    pub prefilled: u64,
    pub generated: u64,
    /// Prompt tokens that came out of the endpoint's cache, all time. Nothing
    /// summed these before: `Usage` reports one request's cache at a time.
    pub cached: u64,
    /// Tokens and seconds behind the average, kept as sums so the average is
    /// over every request ever rather than over the samples still in the ring.
    pub prefill_tokens: u64,
    pub prefill_seconds: f64,
    pub decode_tokens: u64,
    pub decode_seconds: f64,
    /// One tokens-per-second reading per request, oldest first.
    pub prefill_rates: Vec<f32>,
    pub decode_rates: Vec<f32>,
}

/// Where the file lives: beside `no0b.conf`, under the same rules.
pub fn path() -> Option<PathBuf> {
    Some(crate::config::path()?.with_file_name("no0b.totals"))
}

impl Totals {
    /// Read the file. A missing or unreadable one is a first run, not an error.
    ///
    /// The file written under the old name is taken over on the way in: the
    /// all-time totals are the one reading in the window that cannot be
    /// recovered once it is lost, and a rename is no reason to zero them.
    pub fn load(path: &Path) -> Totals {
        crate::config::adopt_legacy(path);
        match std::fs::read_to_string(path) {
            Ok(text) => Totals::parse(&text),
            Err(_) => Totals::default(),
        }
    }

    /// `key = value` per line, `#` a comment. The same format as the settings
    /// file, so there is one format in this program rather than two.
    pub fn parse(text: &str) -> Totals {
        let mut out = Totals::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "prefilled" => out.prefilled = count(value),
                "generated" => out.generated = count(value),
                "cached" => out.cached = count(value),
                "prefill_tokens" => out.prefill_tokens = count(value),
                "prefill_seconds" => out.prefill_seconds = seconds(value),
                "decode_tokens" => out.decode_tokens = count(value),
                "decode_seconds" => out.decode_seconds = seconds(value),
                "prefill_rates" => out.prefill_rates = rates(value),
                "decode_rates" => out.decode_rates = rates(value),
                _ => {}
            }
        }
        out
    }

    /// The file's text, ready to write.
    fn to_text(&self) -> String {
        let list = |rates: &[f32]| {
            rates
                .iter()
                .map(|rate| format!("{rate:.3}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "# NO0B running totals, across every session. Delete to start over.\n\
             prefilled = {}\n\
             generated = {}\n\
             cached = {}\n\
             prefill_tokens = {}\n\
             prefill_seconds = {:.3}\n\
             decode_tokens = {}\n\
             decode_seconds = {:.3}\n\
             prefill_rates = {}\n\
             decode_rates = {}\n",
            self.prefilled,
            self.generated,
            self.cached,
            self.prefill_tokens,
            self.prefill_seconds,
            self.decode_tokens,
            self.decode_seconds,
            list(&self.prefill_rates),
            list(&self.decode_rates),
        )
    }

    /// Replace the file, by rename. Shares the settings writer's path so the
    /// two cannot differ on what a safe write is.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::config::replace_file(path, &self.to_text())
    }

    /// These totals with one live session added, which is what the monitor
    /// shows and what gets written.
    ///
    /// Not in place: the loaded copy has to stay the sessions that came before,
    /// or saving twice in one session would count that session twice.
    pub fn plus(&self, state: &crate::state::State) -> Totals {
        let mut out = self.clone();
        out.prefilled = out.prefilled.saturating_add(state.prefilled);
        out.generated = out.generated.saturating_add(state.generated);
        out.cached = out.cached.saturating_add(state.cached_prefill);
        let (tokens, seconds) = state.rates.prefill_sum();
        out.prefill_tokens = out.prefill_tokens.saturating_add(tokens);
        out.prefill_seconds += seconds;
        let (tokens, seconds) = state.rates.decode_sum();
        out.decode_tokens = out.decode_tokens.saturating_add(tokens);
        out.decode_seconds += seconds;
        out.prefill_rates.extend_from_slice(state.rates.prefill_rates());
        out.decode_rates.extend_from_slice(state.rates.decode_rates());
        trim(&mut out.prefill_rates);
        trim(&mut out.decode_rates);
        out
    }

    /// Tokens per second over every request ever, prefill and decode.
    ///
    /// The four reading functions below are the settings panel's ALL TIME block
    /// (`crate::settings`). They had a pane of their own before that, called
    /// OVERALL, which showed these numbers with nothing on it to say they were
    /// not this session and came off for exactly that: the same numbers under a
    /// heading that says what they are is the whole difference.
    pub fn average_prefill(&self) -> f64 {
        per_second(self.prefill_tokens, self.prefill_seconds)
    }

    pub fn average_decode(&self) -> f64 {
        per_second(self.decode_tokens, self.decode_seconds)
    }

    /// The middle request, which is the reading an average cannot give: one
    /// cold start with a huge transcript drags an average down for the rest of
    /// the day, and the median says what a typical request actually did.
    pub fn median_prefill(&self) -> f64 {
        median(&self.prefill_rates)
    }

    pub fn median_decode(&self) -> f64 {
        median(&self.decode_rates)
    }
}

/// Keep the newest [`SAMPLES`] samples, dropping from the front.
fn trim(rates: &mut Vec<f32>) {
    if rates.len() > SAMPLES {
        rates.drain(..rates.len() - SAMPLES);
    }
}

fn count(value: &str) -> u64 {
    value.parse().unwrap_or(0)
}

/// Seconds, refusing anything that would poison an average. A negative or
/// non-finite duration in the file would divide the totals into a NaN, and a
/// NaN on screen is worse than a zero.
fn seconds(value: &str) -> f64 {
    match value.parse::<f64>() {
        Ok(n) if n.is_finite() && n >= 0.0 => n,
        _ => 0.0,
    }
}

/// A comma separated list of rates, skipping anything unreadable rather than
/// dropping the whole line.
fn rates(value: &str) -> Vec<f32> {
    let mut out: Vec<f32> = value
        .split(',')
        .filter_map(|part| part.trim().parse::<f32>().ok())
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .collect();
    trim(&mut out);
    out
}

fn per_second(tokens: u64, seconds: f64) -> f64 {
    if seconds <= 0.0 || !seconds.is_finite() {
        return 0.0;
    }
    tokens as f64 / seconds
}

/// The middle sample, or the mean of the two middle ones. Zero for no samples,
/// which is what the gauge prints as "not measured yet".
fn median(rates: &[f32]) -> f64 {
    if rates.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f32> = rates.to_vec();
    sorted.sort_by(f32::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] as f64 + sorted[middle] as f64) / 2.0
    } else {
        sorted[middle] as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noob_proto::{Event, Usage};

    fn written() -> Totals {
        Totals {
            prefilled: 120_000,
            generated: 9_000,
            cached: 88_000,
            prefill_tokens: 100_000,
            prefill_seconds: 40.0,
            decode_tokens: 9_000,
            decode_seconds: 300.0,
            prefill_rates: vec![2000.0, 2500.0, 3000.0],
            decode_rates: vec![30.0, 31.0],
        }
    }

    /// What is written comes back. Anything else is a totals file that silently
    /// resets itself every time the window opens.
    #[test]
    fn a_file_this_writes_is_a_file_this_reads() {
        let totals = written();
        let back = Totals::parse(&totals.to_text());
        assert_eq!(back, totals);
    }

    /// First run, and a file somebody scribbled in. Both have to produce a
    /// window, and neither may produce a NaN.
    #[test]
    fn a_missing_or_corrupt_file_reads_as_nothing_recorded() {
        assert_eq!(Totals::parse(""), Totals::default());
        assert_eq!(
            Totals::load(Path::new("/nonexistent/no0b.totals")),
            Totals::default()
        );
        let junk = Totals::parse(
            "prefilled = twelve\n\
             generated =\n\
             decode_seconds = -3\n\
             prefill_seconds = NaN\n\
             cached = 40\n\
             something_else = 9\n\
             a line with no equals\n\
             decode_rates = 12.5,,oops,-4,25\n",
        );
        assert_eq!(junk.prefilled, 0);
        assert_eq!(junk.cached, 40, "the readable lines still land");
        assert_eq!(junk.decode_seconds, 0.0);
        assert_eq!(junk.prefill_seconds, 0.0);
        assert_eq!(junk.decode_rates, vec![12.5, 25.0], "the junk is skipped");
        for reading in [
            junk.average_prefill(),
            junk.average_decode(),
            junk.median_prefill(),
            junk.median_decode(),
        ] {
            assert!(reading.is_finite(), "{reading}");
        }
    }

    /// Written by rename, and it survives a round trip through the filesystem.
    #[test]
    fn saving_and_loading_keeps_every_number() {
        let dir = std::env::temp_dir().join(format!("no0b-totals-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("no0b.totals");
        written().save(&path).expect("saved");
        assert_eq!(Totals::load(&path), written());
        // And again over the top of itself, which is what every save after the
        // first one is.
        let mut more = written();
        more.generated += 5;
        more.save(&path).expect("saved again");
        assert_eq!(Totals::load(&path).generated, written().generated + 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The totals are the one reading in the window that cannot be measured
    /// again: they are every session that ever ran. A rename that dropped them
    /// would set the counters somebody has been watching for weeks back to zero
    /// and there would be no way back.
    #[test]
    fn totals_written_under_the_old_name_are_carried_over() {
        let dir = std::env::temp_dir().join(format!("no0b-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let (legacy, current) = (dir.join("clippy.totals"), dir.join("no0b.totals"));
        written().save(&legacy).expect("saved under the old name");

        assert_eq!(Totals::load(&current), written());
        assert!(current.exists(), "the file was not moved");
        assert!(!legacy.exists(), "the old name is still there");

        // Both names present is not a migration: the current file is the one
        // being written and it wins, with the old one left where it is.
        let mut newer = written();
        newer.generated += 11;
        newer.save(&current).expect("saved");
        written().save(&legacy).expect("saved under the old name again");
        assert_eq!(Totals::load(&current).generated, written().generated + 11);
        assert!(legacy.exists(), "the old file was deleted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_totals_file_sits_beside_the_settings_file() {
        // Only meaningful when there is a home to put it in, which there is in
        // this suite; the point is the two are one directory apart from never.
        if let (Some(config), Some(totals)) = (crate::config::path(), path()) {
            assert_eq!(config.parent(), totals.parent());
            assert_ne!(config, totals);
        }
    }

    /// The average is over every request ever and the median over the samples
    /// kept. An even count of samples is the mean of the middle two.
    #[test]
    fn the_average_is_the_sums_and_the_median_is_the_middle_sample() {
        let totals = written();
        assert_eq!(totals.average_prefill(), 2500.0);
        assert_eq!(totals.average_decode(), 30.0);
        assert_eq!(totals.median_prefill(), 2500.0, "three samples, the middle");
        assert_eq!(totals.median_decode(), 30.5, "two samples, the mean of them");
        // Nothing measured says nothing rather than dividing by zero.
        let empty = Totals::default();
        assert_eq!(empty.average_decode(), 0.0);
        assert_eq!(empty.median_decode(), 0.0);
    }

    /// A median is not an average: one cold request must not move it.
    #[test]
    fn a_single_slow_request_moves_the_average_and_not_the_median() {
        let totals = Totals {
            decode_tokens: 1_030,
            decode_seconds: 40.0,
            decode_rates: vec![30.0, 31.0, 30.5, 1.0],
            ..Totals::default()
        };
        assert!(totals.average_decode() < 30.0, "{}", totals.average_decode());
        assert_eq!(totals.median_decode(), 30.25);
    }

    /// The ring is bounded, and what falls out is the oldest.
    #[test]
    fn the_sample_ring_keeps_the_newest_and_is_bounded() {
        let mut totals = Totals {
            decode_rates: (0..SAMPLES as u32 + 40).map(|n| n as f32 + 1.0).collect(),
            ..Totals::default()
        };
        trim(&mut totals.decode_rates);
        assert_eq!(totals.decode_rates.len(), SAMPLES);
        assert_eq!(totals.decode_rates[0], 41.0, "the oldest 40 fell off");
        // And through the file, which is the path that actually has to hold.
        let back = Totals::parse(&totals.to_text());
        assert_eq!(back.decode_rates.len(), SAMPLES);
    }

    /// The live session is added to the file's numbers, and adding it twice
    /// does not count it twice: `plus` never touches what it was called on.
    #[test]
    fn a_session_is_added_to_what_came_before_and_only_once() {
        let carried = Totals {
            prefilled: 1_000,
            generated: 100,
            cached: 500,
            ..Totals::default()
        };
        let mut state = crate::state::State::new();
        state.apply_at(Event::TurnStart { turn: 1 }, Some(0.0));
        state.apply_at(Event::TextDelta { d: "hi".into() }, Some(1.0));
        state.apply_at(
            Event::UsageReport {
                usage: Usage {
                    prompt: 900,
                    cached_prompt: 400,
                    completion: 60,
                    context_total: 65_536,
                },
            },
            Some(3.0),
        );

        let once = carried.plus(&state);
        assert_eq!(once.prefilled, 1_500, "500 prefilled this session");
        assert_eq!(once.generated, 160);
        assert_eq!(once.cached, 900);
        assert_eq!(once.prefill_rates.len(), 1);
        assert_eq!(once.decode_rates.len(), 1);
        assert!(once.average_decode() > 0.0);
        // The carried copy is untouched, so the second write is the same file.
        assert_eq!(carried.prefilled, 1_000);
        assert_eq!(carried.plus(&state), once);
    }
}
