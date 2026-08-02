//! The AGENT section: what the CLI is pointed at, out of the file the CLI
//! owns.
//!
//! One of the settings panel's nested section boxes. It builds rows out of the
//! shared vocabulary in [`crate::settings`] and owns the plain-words names for
//! the agent's env keys; the frame owns the cursor and the writes.

use crate::agent::{self, Agent};
use crate::settings::{Card, CardField, Doing, File, Kind, Row, SECRET, UNSET};

/// What the connection card says before anybody has pressed its button. Not a
/// verdict: the window has asked nothing yet, and a card that said "ok" off no
/// evidence would be the worst line on the panel.
const NOT_CHECKED: &str = "not checked yet";

/// The keys of the agent's file this section draws a field of its own for: what
/// to call each one in plain words, and the short line under it.
///
/// The label is the plain words and the key together, `rounds per input
/// (NOOB_MAX_ROUNDS)`, because both are wanted at once: the words say what the
/// control does and the key says which line it writes. The line under it is
/// only what neither of those says, which on most of these is nothing at all.
const AGENT_FIELDS: [(&str, &str, &str); 11] = [
    (agent::ENDPOINT, "endpoint", ""),
    (agent::API_KEY, "api key", ""),
    (agent::MODEL, "model", "as the endpoint names it"),
    (agent::API_STYLE, "api style", "chat or responses"),
    (agent::REASONING, "reasoning", "on or off"),
    (agent::CTX, "context window", "tokens before a session compacts"),
    (agent::MAX_ROUNDS, "rounds per input", "0 for no limit"),
    (agent::TASK_CONCURRENCY, "max sub-agents", "1 to 64"),
    (agent::TASK_MAX_TURNS, "sub-agent rounds", "0 for no limit"),
    (agent::TASK_TOOLS, "sub-agent tools", "read-only, web, or all"),
    (agent::TASK_WALL_CLOCK, "sub-agent seconds", "0 for no limit"),
];

/// What to call one of the agent's keys, and what to say under it. The key
/// itself for anything not on [`AGENT_FIELDS`], which is the honest label for a
/// line this window knows nothing about.
fn agent_says(key: &str) -> (&str, &'static str) {
    match AGENT_FIELDS.iter().find(|(known, ..)| *known == key) {
        Some((_, label, hint)) => (label, hint),
        None => (key, ""),
    }
}

/// The label a field carries: the plain words with the key it writes after
/// them.
fn titled(key: &str) -> String {
    match agent_says(key).0 {
        words if words == key => String::from(key),
        words => format!("{words} ({key})"),
    }
}

/// The line under a field: what it means, and whether anybody has set it.
///
/// Two short facts at most, because that is all there is to say once the label
/// carries the key: a value nobody wrote is the CLI's own default, and a budget
/// where 0 means no limit says so.
fn under(key: &str, set: bool) -> Option<String> {
    let mut parts = Vec::new();
    if !set {
        parts.push("not set: the default");
    }
    match agent_says(key).1 {
        "" => {}
        line => parts.push(line),
    }
    match parts.is_empty() {
        true => None,
        false => Some(parts.join(", ")),
    }
}

/// Whether the section already draws a field for a key, which is what keeps the
/// last card the rest of the file rather than the whole of it twice.
fn agent_has_a_field(key: &str) -> bool {
    AGENT_FIELDS.iter().any(|(known, ..)| *known == key)
}

/// The two settings of the agent's own file that are numbers with a range, so
/// the panel can offer them as tracks instead of asking for a number to be
/// typed: the context window the CLI budgets against, and how many sub-agent
/// tasks it runs at once.
///
/// The bounds are the CLI's own ([`crate::agent`] reads them off it), so the
/// right end of the concurrency track is the maximum the agent will honour and
/// there is nothing to guess. Every other key in that file is listed as a
/// reading, because the window does not know what the CLI would accept for it.
pub(crate) const AGENT_SETTINGS: [(&str, Kind); 8] = [
    (agent::API_STYLE, Kind::Choice(&CHOICE_API_STYLE)),
    (agent::REASONING, Kind::Choice(&CHOICE_REASONING)),
    (
        agent::CTX,
        Kind::Number {
            step: agent::CTX_STEP,
            low: agent::CTX_LOW,
            high: agent::CTX_HIGH,
            places: 0,
            stops: &agent::CTX_STOPS,
        },
    ),
    (
        agent::MAX_ROUNDS,
        Kind::Number {
            step: agent::ROUNDS_STEP,
            low: agent::ROUNDS_LOW,
            high: agent::ROUNDS_HIGH,
            places: 0,
            stops: &agent::ROUNDS_STOPS,
        },
    ),
    (
        agent::TASK_CONCURRENCY,
        Kind::Number {
            step: agent::TASK_CONCURRENCY_STEP,
            low: agent::TASK_CONCURRENCY_LOW,
            high: agent::TASK_CONCURRENCY_HIGH,
            places: 0,
            stops: &agent::TASK_CONCURRENCY_STOPS,
        },
    ),
    (
        agent::TASK_MAX_TURNS,
        Kind::Number {
            step: agent::ROUNDS_STEP,
            low: agent::ROUNDS_LOW,
            high: agent::ROUNDS_HIGH,
            places: 0,
            stops: &agent::ROUNDS_STOPS,
        },
    ),
    (agent::TASK_TOOLS, Kind::Choice(&CHOICE_TASK_TOOLS)),
    (
        agent::TASK_WALL_CLOCK,
        Kind::Number {
            step: agent::WALL_CLOCK_STEP,
            low: agent::WALL_CLOCK_LOW,
            high: agent::WALL_CLOCK_HIGH,
            places: 0,
            stops: &agent::WALL_CLOCK_STOPS,
        },
    ),
];

/// [`agent::TASK_TOOLS_CHOICES`] with the static shape `Kind::Choice` wants.
const CHOICE_TASK_TOOLS: [&str; 3] = agent::TASK_TOOLS_CHOICES;

/// The two request shapes `noob-provider` speaks, and the thinking switch.
/// Both are what the CLI refuses anything else for, so the list is the whole
/// of what can be written. Neither has a default to show: unset, the CLI picks
/// the shape by the address and leaves the thinking to the server's own flags,
/// so an unset row reads [`UNSET`] and the first nudge sets it.
const CHOICE_API_STYLE: [&str; 2] = ["chat", "responses"];
const CHOICE_REASONING: [&str; 2] = ["on", "off"];

/// What the CLI uses for one of its own settings when the file does not carry
/// it. Read off the CLI rather than chosen here: a row that shows a number the
/// agent is not actually running with is worse than no row.
fn agent_default(key: &str) -> String {
    match key {
        agent::CTX => agent::CTX_DEFAULT.to_string(),
        agent::TASK_CONCURRENCY => agent::TASK_CONCURRENCY_DEFAULT.to_string(),
        agent::MAX_ROUNDS | agent::TASK_MAX_TURNS => agent::ROUNDS_DEFAULT.to_string(),
        agent::TASK_TOOLS => agent::TASK_TOOLS_DEFAULT.to_string(),
        agent::TASK_WALL_CLOCK => agent::WALL_CLOCK_DEFAULT.to_string(),
        // Unreachable through AGENT_SETTINGS, and a number is the honest answer
        // for a row that says it is one.
        _ => String::from("0"),
    }
}

/// What the agent is pointed at, out of the file the CLI owns.
///
/// Cards, in the order somebody meeting this window needs them: where the
/// model is and whether it answers, the credential, which model it is, how
/// much the agent gets, the file all of it is written in, and whatever else
/// that file carries.
///
/// Every field is one label over one value, and the label carries both the
/// plain words and the key it writes: `rounds per input (NOOB_MAX_ROUNDS)`.
/// The line under a field is only what neither of those says, which is
/// usually nothing. Everything the CLI accepts a value for is set from here:
/// the two request shapes and the thinking switch are lists, the budgets are
/// tracks, and the endpoint and the model are typed into.
///
/// `health` is what the last connection check answered, and `show_key`
/// whether the credential is being looked at; both live in the frame, since
/// one comes off a process and the other off a button.
pub fn rows(agent: &Agent, health: Option<&str>, show_key: bool) -> Vec<Row> {
    let mut controls = Vec::new();
    for (key, kind) in AGENT_SETTINGS {
        let set = agent.setting(key).is_some();
        let value = match agent.setting(key) {
            Some(value) => value.to_string(),
            // A list with no default has nothing honest to show but that
            // nobody has set it; a track has to stand somewhere, so it stands
            // on the number the CLI would use.
            None => match kind {
                Kind::Choice(_) => String::from(UNSET),
                Kind::Number { .. } => agent_default(key),
            },
        };
        let field = CardField::setting(&titled(key), key, value, kind, File::Agent);
        controls.push(match under(key, set) {
            Some(line) => field.saying(line.as_str()),
            None => field,
        });
    }
    // AGENT_SETTINGS order. A card carries at most two controls, since two is
    // what a press can name, so the eight are four pairs.
    let mut it = controls.into_iter();
    let mut next = || it.next().expect("eight controls");
    let (api_style, reasoning) = (next(), next());
    let (ctx, rounds) = (next(), next());
    let (tasks, task_rounds) = (next(), next());
    let (task_tools, wall_clock) = (next(), next());
    let mut rows = vec![
        // The endpoint first, because an agent pointed at nothing is the one
        // state this window cannot work in, with the shape of the requests it
        // sends beside it and, under both, whether the thing at the other end
        // actually answered.
        Row::Card(Card {
            title: String::from("CONNECTION"),
            fields: vec![
                CardField::text(
                    &titled(agent::ENDPOINT),
                    agent::ENDPOINT,
                    agent.endpoint().unwrap_or_default().to_string(),
                ),
                api_style,
                CardField::reading("answered", String::from(health.unwrap_or(NOT_CHECKED))),
            ],
            hint: Some(String::from(
                "empty endpoint: the CLI probes :8080 :8090 :11434 :1234 :8000. check writes what is typed, then asks",
            )),
            does: Some(Doing::Check),
        }),
        // The way back, under the field it writes and shaped like every other
        // restore on this panel: its own card, its own button, and the value
        // it writes said in words rather than left to be discovered.
        Row::Card(Card {
            title: String::from("BACK TO THE DEFAULT ENDPOINT"),
            fields: Vec::new(),
            hint: Some(format!(
                "writes {}, llama.cpp's own port and the first address the CLI probes",
                agent::ENDPOINT_DEFAULT
            )),
            does: Some(Doing::DefaultEndpoint),
        }),
        // The credential on a card of its own, because the one button it has
        // is the one that shows it.
        Row::Card(Card {
            title: String::from("CREDENTIAL"),
            fields: vec![CardField::reading(
                &titled(agent::API_KEY),
                key_says(agent, show_key),
            )],
            hint: Some(String::from("sent as the bearer token")),
            does: Some(match show_key {
                true => Doing::Hide,
                false => Doing::Reveal,
            }),
        }),
        Row::Card(Card {
            title: String::from("MODEL"),
            fields: vec![
                CardField::text(
                    &titled(agent::MODEL),
                    agent::MODEL,
                    agent.setting(agent::MODEL).unwrap_or_default().to_string(),
                )
                .saying(
                    under(agent::MODEL, agent.setting(agent::MODEL).is_some())
                        .unwrap_or_default()
                        .as_str(),
                ),
                reasoning,
            ],
            hint: None,
            does: None,
        }),
        Row::Card(Card {
            title: String::from("LIMITS"),
            fields: vec![ctx, rounds],
            hint: None,
            does: None,
        }),
        // The fleet on cards of its own: how many children and what each may
        // touch, then the two budgets that stop one.
        Row::Card(Card {
            title: String::from("MULTI-AGENT"),
            fields: vec![tasks, task_tools],
            hint: None,
            does: None,
        }),
        Row::Card(Card {
            title: String::from("MULTI-AGENT BUDGETS"),
            fields: vec![task_rounds, wall_clock],
            hint: None,
            does: None,
        }),
        Row::Card(Card {
            title: String::from("THE SETTINGS FILE"),
            fields: vec![
                CardField::reading(
                    "file",
                    match (&agent.env_path, agent.env_exists) {
                        // Not there yet is worth saying: an agent configured
                        // entirely by environment has no file at all, and the
                        // first save writes one.
                        (Some(path), false) => format!("{} (not there yet)", path.display()),
                        (Some(path), true) => path.display().to_string(),
                        (None, _) => {
                            String::from("nowhere: no config directory to read one in")
                        }
                    },
                )
                .saying("one KEY=value to a line; the CLI re-reads it on every request"),
            ],
            hint: None,
            does: None,
        }),
    ];
    // Whatever else the file carries, under one title rather than as loose
    // rows: a key this window has never heard of is still a key the agent
    // reads, and a section that dropped it would be a window claiming the
    // file says less than it does.
    let rest: Vec<CardField> = agent
        .env
        .iter()
        .filter(|(key, _)| !agent_has_a_field(key))
        .map(|(key, _)| CardField::reading(key, env_says(agent, key)))
        .collect();
    if !rest.is_empty() {
        rows.push(Row::Card(Card {
            title: String::from("THE REST OF THE FILE"),
            fields: rest,
            hint: Some(String::from(
                "keys the CLI reads that this window has no control for: edit them in the file",
            )),
            does: None,
        }));
    }
    rows
}

/// What the credential field shows: nothing anybody set, the value itself
/// while it is being looked at, or a run of dots the length of it.
///
/// Dots rather than the value, because a settings panel is on a screen
/// somebody else can be standing behind; the button beside it is how it is
/// read out when that is not the case.
fn key_says(agent: &Agent, show: bool) -> String {
    match agent.env.iter().find(|(known, _)| known == agent::API_KEY) {
        None => String::from(UNSET),
        Some((_, value)) if show => value.clone(),
        Some((_, value)) => "\u{2022}".repeat(value.chars().count().clamp(8, 32)),
    }
}

/// What the agent's file says for one key, said the way this panel says it.
///
/// A credential is `set, and not shown here`; a key the file does not carry
/// reads [`UNSET`], which is a field that says nobody has set it rather than
/// an empty line that reads as a value that failed to load.
fn env_says(agent: &Agent, key: &str) -> String {
    match agent.env.iter().find(|(known, _)| known == key) {
        Some(_) if agent::is_secret(key) => String::from(SECRET),
        Some((_, value)) => value.clone(),
        None => String::from(UNSET),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::settings::testing::*;
    use crate::settings::{
        card_is_reachable, commit, landable, write_endpoint, Change, Settings, Side, AGENT,
    };
    use std::path::Path;

    /// The agent section reads the CLI's own file: where it is, what it points
    /// at, and what else is set, with no credential anywhere on it.
    #[test]
    fn the_agent_section_says_what_the_cli_is_pointed_at() {
        let dir = scratch_dir("agent");
        std::fs::write(
            dir.join(".env"),
            "NOOB_BASE_URL=http://localhost:8080/v1\nNOOB_CTX=262144\nNOOB_API_KEY=sk-secret\n",
        )
        .expect("a file");
        let agent = Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel =
            Settings::open(&Config::default(), Some(Path::new("/tmp/no0b.conf")), agent);
        go_to(&mut panel, AGENT);
        let text = said(&panel);
        assert!(
            text.contains(&dir.join(".env").display().to_string()),
            "the panel does not say where the file is: {text}"
        );
        assert!(text.contains("http://localhost:8080/v1"), "{text}");
        // The number is under a name anybody can read, and the key that writes
        // it is in that name: both are wanted at once, and a sentence under
        // every field to carry the second one was the pile of prose this
        // section was.
        assert!(text.contains("context window (NOOB_CTX) 262144"), "{text}");
        // The concurrency row is named for what it caps, not for a phrase
        // nobody could map to the key.
        assert!(
            text.contains("max sub-agents (NOOB_TASK_CONCURRENCY)"),
            "{text}"
        );
        assert_eq!(panel.agent_file(), Some(dir.join(".env").as_path()));

        // The credential is dots until its own button is pressed, and the
        // value itself after it. Nothing about it is remembered: the panel
        // opens covered.
        assert!(!panel.key_shown());
        assert!(!text.contains("sk-secret"), "a credential is on the panel: {text}");
        assert!(text.contains("api key (NOOB_API_KEY) \u{2022}"), "{text}");
        panel.flip_key(&Config::default());
        go_to(&mut panel, AGENT);
        assert!(said(&panel).contains("sk-secret"), "the button showed nothing");
        panel.flip_key(&Config::default());
        go_to(&mut panel, AGENT);
        assert!(!said(&panel).contains("sk-secret"), "it never went back");

        // What the connection check answered lands on the card that asked.
        assert!(said(&panel).contains("not checked yet"));
        panel.adopt_health(
            String::from("http://localhost:8080/v1 answers /models (HTTP 200)"),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        assert!(said(&panel).contains("answers /models (HTTP 200)"), "{text}");

        // The endpoint is the one thing here that is typed into rather than
        // nudged, and it is where the section opens: the first field of the
        // first card.
        assert!(panel.on_row());
        assert_eq!(panel.cursor(), 0, "the section does not open on the endpoint");
        assert_eq!(panel.side(), Side::Left);
        assert!(
            matches!(panel.at_cursor(), Some(Row::Field { key, .. }) if *key == agent::ENDPOINT),
            "{:?}",
            panel.at_cursor()
        );
        assert!(panel.edit());
        assert_eq!(panel.editing(), Some("http://localhost:8080/v1"));
        assert!(!panel.edit(), "already editing");
        assert!(panel.backspace());
        assert!(panel.type_text("2\n\t "), "whitespace is not typed");
        assert_eq!(panel.editing(), Some("http://localhost:8080/v2"));
        assert!(panel.hint().contains("enter saves"), "{}", panel.hint());
        // Nothing has been written: the row still says what the file says.
        assert!(
            said(&panel).contains("http://localhost:8080/v1"),
            "the edit reached the row early"
        );
        assert!(panel.cancel_edit());
        assert!(!panel.cancel_edit());

        // And the whole way through: type it, save it, read it back.
        assert!(panel.edit());
        assert!(panel.type_text("2"));
        let (key, typed) = panel.finish_edit().expect("something was typed");
        assert_eq!(key, agent::ENDPOINT);
        write_endpoint(&dir.join(".env"), key, &typed).expect("the file takes it");
        panel.adopt_agent(
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        assert!(
            panel.all_rows().any(|(_, row)| matches!(
                row,
                Row::Field { value, .. } if value == "http://localhost:8080/v12"
            )),
            "{:?}",
            panel.rows()
        );
        // The rest of the file survived the write, which is the whole point of
        // going through the agent's own writer.
        let after = std::fs::read_to_string(dir.join(".env")).expect("the file");
        assert!(after.contains("NOOB_CTX=262144"), "{after}");
        assert!(after.contains("NOOB_API_KEY=sk-secret"), "{after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two numbers that decide what the agent actually gets are controls on
    /// its own section: read off the CLI's file, held to the CLI's own bounds,
    /// nudged and dragged the way every other setting is, and written back into
    /// that file rather than the window's.
    #[test]
    fn the_agent_s_context_and_task_concurrency_are_set_on_the_panel() {
        let dir = scratch_dir("agent-numbers");
        let env = dir.join(".env");
        std::fs::write(
            &env,
            "NOOB_BASE_URL=http://localhost:8080/v1\nNOOB_CTX=262144\nNOOB_TASK_CONCURRENCY=2   # two at a time\n",
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), Some(Path::new("/tmp/no0b.conf")), read());

        // What the file says, on the agent's section, once each: a row that is
        // also listed as a reading is the same setting twice with only one of
        // them doing anything.
        put_cursor(&mut panel, agent::CTX);
        assert_eq!(panel.title(), AGENT, "the cursor is not on the agent's section");
        assert_eq!(value(&panel, agent::CTX), "262144");
        assert_eq!(value(&panel, agent::TASK_CONCURRENCY), "2");
        let text = said(&panel);
        assert_eq!(text.matches(agent::CTX).count(), 1, "{text}");
        assert_eq!(text.matches(agent::TASK_CONCURRENCY).count(), 1, "{text}");

        // A nudge steps by the CLI's own unit and says which file it belongs in.
        assert_eq!(
            panel.change(true).expect("the context window nudges"),
            Change {
                key: agent::CTX,
                value: String::from("266240"),
                file: File::Agent,
            }
        );

        // Both ends of the concurrency track are the CLI's own: one at the
        // bottom, and at the top the sixty-four it caps itself at, so the
        // maximum is somewhere the pointer can be dropped rather than a
        // number to guess.
        put_cursor(&mut panel, agent::TASK_CONCURRENCY);
        let at = panel.cursor();
        assert!(panel.slide(at, panel.side(), 0.0));
        assert_eq!(panel.preview(at, panel.side()), Some("1"));
        assert!(panel.slide(at, panel.side(), 1.0));
        let most = panel.drop_slider().expect("the drag decided something");
        assert_eq!(
            most,
            Change {
                key: agent::TASK_CONCURRENCY,
                value: String::from("64"),
                file: File::Agent,
            }
        );
        // And the context window bottoms out where the CLI stops reading it.
        put_cursor(&mut panel, agent::CTX);
        let at = panel.cursor();
        assert!(panel.slide(at, panel.side(), 0.0));
        assert_eq!(panel.preview(at, panel.side()), Some("4096"));
        panel.drop_slider();

        // Written, it lands in the agent's file, the line keeps its comment and
        // nothing else in the file moves.
        write_endpoint(&env, most.key, &most.value).expect("the file takes it");
        panel.adopt_agent(read(), &Config::default());
        assert_eq!(value(&panel, agent::TASK_CONCURRENCY), "64");
        let after = std::fs::read_to_string(&env).expect("the file");
        assert!(after.contains("NOOB_TASK_CONCURRENCY=64"), "{after}");
        assert!(after.contains("# two at a time"), "the comment is gone: {after}");
        assert!(after.contains("NOOB_CTX=262144"), "{after}");
        assert!(
            after.contains("NOOB_BASE_URL=http://localhost:8080/v1"),
            "{after}"
        );

        // The two files are not interchangeable, which is why a change carries
        // the answer: the window's writer refuses a key of the agent's outright
        // rather than adding a line the window will never read.
        let wrong = commit(
            Path::new("/tmp/no0b.conf"),
            &Change {
                key: agent::CTX,
                value: String::from("8192"),
                file: File::Agent,
            },
        );
        assert!(wrong.is_err(), "the window's file took a setting of the agent's");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two tracks are magnetic: a drag passing near a checkpoint snaps to
    /// it, each checkpoint is a tick on the track, and the keyboard nudge
    /// still steps by the CLI's own unit rather than jumping detent to detent.
    #[test]
    fn the_agent_sliders_snap_to_their_checkpoints() {
        let dir = scratch_dir("agent-detents");
        std::fs::write(dir.join(".env"), "NOOB_CTX=131072\nNOOB_TASK_CONCURRENCY=2\n")
            .expect("a file");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );

        // The context window: a drag two steps past 128k lands on 128k.
        put_cursor(&mut panel, agent::CTX);
        let at = panel.cursor();
        assert!(panel.slide(at, panel.side(), 0.13));
        assert_eq!(panel.preview(at, panel.side()), Some("131072"));
        // Far from every checkpoint the plain step is the only snap.
        assert!(panel.slide(at, panel.side(), 0.5));
        let free = panel.preview(at, panel.side()).expect("a value");
        assert!(
            !agent::CTX_STOPS.iter().any(|stop| free == stop.to_string()),
            "{free} snapped with no detent near"
        );
        panel.drop_slider();
        // The nudge is the CLI's own step, from a detent as from anywhere.
        assert_eq!(
            panel.change(true).expect("a nudge").value,
            "135168",
            "a detent bent the arrow keys"
        );

        // The concurrency track: near sixteen means sixteen. At the narrow
        // snap window the magnet on an integer track is gentle by design; the
        // context track above is where snapping visibly beats the step.
        put_cursor(&mut panel, agent::TASK_CONCURRENCY);
        let at = panel.cursor();
        assert!(panel.slide(at, panel.side(), 0.26));
        assert_eq!(panel.preview(at, panel.side()), Some("16"));
        panel.drop_slider();

        // And every checkpoint is a tick on its track, in track order. The
        // one choice among the controls has no track to tick.
        for (key, kind) in AGENT_SETTINGS {
            let Kind::Number { stops, .. } = kind else {
                continue;
            };
            let ticks = kind.stop_fractions();
            assert_eq!(ticks.len(), stops.len(), "{key} loses ticks");
            assert!(
                ticks.windows(2).all(|pair| pair[0] < pair[1]),
                "{key}: the ticks are out of order"
            );
            assert!(
                ticks.iter().all(|tick| (0.0..=1.0).contains(tick)),
                "{key}: a tick is off the track"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With neither of them in the file the rows read what the CLI falls back
    /// to, and the section says that is what they are. A slider showing a number
    /// nobody wrote, with nothing saying so, is a window inventing a setting.
    #[test]
    fn the_agent_s_numbers_read_as_the_cli_s_defaults_until_they_are_written() {
        let dir = scratch_dir("agent-unset");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, AGENT);
        assert_eq!(value(&panel, agent::CTX), agent::CTX_DEFAULT.to_string());
        assert_eq!(
            value(&panel, agent::TASK_CONCURRENCY),
            agent::TASK_CONCURRENCY_DEFAULT.to_string()
        );
        let text = said(&panel);
        assert!(text.contains("not set: the default"), "{text}");
        assert!(text.contains(agent::CTX), "{text}");
        assert!(text.contains(agent::TASK_CONCURRENCY), "{text}");
        // A list with no default has nothing to stand on, so it says so rather
        // than showing one of its names as though somebody chose it.
        assert_eq!(value(&panel, agent::API_STYLE), UNSET);
        assert_eq!(value(&panel, agent::REASONING), UNSET);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "actually is awful as is now, unclear because has too many lines
    /// between": the section is cards, every field of every one of them is a
    /// label with a sentence under it, and the keyboard reaches everything that
    /// can be set.
    ///
    /// It was a two column form of raw environment keys with notes standing
    /// between the rows. Left and right are the nudge on a control, so they
    /// cannot also be how a card is crossed: the shifted arrow is, and it points
    /// at the field it lands on.
    #[test]
    fn the_agent_is_cards_the_keyboard_can_walk() {
        let dir = scratch_dir("agent-cards");
        std::fs::write(
            dir.join(".env"),
            "NOOB_BASE_URL=http://localhost:8080/v1\nNOOB_CTX=262144\nNOOB_TASK_CONCURRENCY=2\n",
        )
        .expect("a file");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, AGENT);

        // Cards, in the order somebody needs them.
        let titles: Vec<String> = panel
            .rows()
            .iter()
            .map(|row| match row {
                Row::Card(card) => card.title.clone(),
                other => panic!("the section carries a loose row: {other:?}"),
            })
            .collect();
        assert_eq!(
            titles,
            [
                "CONNECTION",
                "BACK TO THE DEFAULT ENDPOINT",
                "CREDENTIAL",
                "MODEL",
                "LIMITS",
                "MULTI-AGENT",
                "MULTI-AGENT BUDGETS",
                "THE SETTINGS FILE"
            ],
            "{titles:?}"
        );

        // Every field is a label nobody has to have read the CLI to understand,
        // with the key and what it decides in one sentence under it. A row that
        // says `NOOB_TASK_CONCURRENCY 4` and nothing else is the complaint.
        for row in panel.rows() {
            let Row::Card(card) = row else {
                continue;
            };
            assert!(
                card_is_reachable(card),
                "{}: a field that can be set is past the two a press can name",
                card.title
            );
            for field in &card.fields {
                assert!(!field.label.is_empty(), "{}: a field has no name", card.title);
                // Plain words first and the key after them, never the key on
                // its own: `NOOB_TASK_CONCURRENCY 4` was the complaint, and a
                // label with no key in it was the half-answer to it.
                assert!(
                    field.label.starts_with(|ch: char| ch.is_ascii_lowercase()),
                    "{}: {} leads with something other than what it is",
                    card.title,
                    field.label
                );
                let hint = field.hint.clone().unwrap_or_default();
                assert!(
                    !hint.is_empty() || card.hint.is_some(),
                    "{}: {} says nothing about what it is",
                    card.title,
                    field.label
                );
            }
        }
        // The three keys nobody could act on before are named in the sentences,
        // so the field says which line of the file writes it.
        let text = said(&panel);
        for key in [
            agent::ENDPOINT,
            agent::API_KEY,
            agent::MODEL,
            agent::CTX,
            agent::TASK_CONCURRENCY,
        ] {
            assert!(text.contains(key), "{key} is nowhere on the section: {text}");
        }

        // It opens on the endpoint, the shifted arrow crosses to the field
        // beside it, and the plain arrow keys still nudge rather than moving on.
        assert_eq!((panel.cursor(), panel.side()), (0, Side::Left));
        assert!(matches!(panel.at_cursor(), Some(Row::Field { .. })));
        assert!(panel.hint().contains("enter edits it"), "{}", panel.hint());
        // The request shape beside it is a list, so the shifted arrow crosses
        // to it and the plain ones then walk the names.
        assert!(panel.cross(Side::Right), "the endpoint's card has no second half");
        assert!(
            matches!(panel.at_cursor(), Some(Row::Setting { key, .. }) if *key == agent::API_STYLE),
            "{:?}",
            panel.at_cursor()
        );
        assert!(panel.cross(Side::Left));

        // Down to the card of numbers: two fields, both of them tracks, and the
        // shifted arrow crosses between them.
        let numbers = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Card(card) if card.title == "LIMITS"))
            .expect("the card of numbers");
        assert!(panel.point_at(numbers, Side::Left));
        assert!(
            matches!(panel.at_cursor(), Some(Row::Setting { key, .. }) if *key == agent::CTX)
        );
        assert!(
            panel.hint().contains("shift left and right cross"),
            "{}",
            panel.hint()
        );
        assert_eq!(
            panel.change(true).expect("the context window nudges"),
            Change {
                key: agent::CTX,
                value: String::from("266240"),
                file: File::Agent,
            }
        );
        assert!(!panel.cross(Side::Left), "it is already in that field");
        assert!(panel.cross(Side::Right));
        assert!(
            matches!(panel.at_cursor(), Some(Row::Setting { key, .. }) if *key == agent::MAX_ROUNDS)
        );

        // And the cards of readings hold no cursor at all: a row the arrow keys
        // stop on where no key does anything is a dead stop.
        for (at, row) in panel.rows().iter().enumerate() {
            let Row::Card(card) = row else {
                continue;
            };
            // A button counts as something to do: the credential card is read
            // out and its one act is the press that shows it, and a card the
            // keys cannot reach is a button only a pointer can press.
            let doable = card.fields.iter().any(CardField::editable) || card.does.is_some();
            assert_eq!(
                landable(row),
                doable,
                "row {at}, {}, holds the cursor over nothing",
                card.title
            );
        }

        // Up and down walk the cards that can be set, and every one of them is
        // reachable from the top.
        let fleet = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Card(card) if card.title == "MULTI-AGENT"))
            .expect("the multi-agent card");
        assert!(panel.jump(false));
        let mut seen = vec![panel.cursor()];
        while panel.step(true) {
            seen.push(panel.cursor());
        }
        // Every card down to the last budget: the restore's button and the
        // credential's are stops of their own, and the settings file at the
        // end is the one card with nothing to do on it.
        assert_eq!(
            seen,
            vec![0, 1, 2, 3, numbers, fleet, fleet + 1],
            "the keyboard cannot walk the section: {seen:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The way back to the endpoint the CLI would have found on its own.
    ///
    /// It is a card with one button, the shape every other restore on this
    /// panel has, and what it writes is llama.cpp's own port: the first
    /// address `autodetect_base_url` probes, so an agent that has never been
    /// pointed anywhere ends up there too.
    #[test]
    fn the_endpoint_has_a_way_back_to_the_default() {
        let dir = scratch_dir("agent-default-endpoint");
        let env = dir.join(".env");
        std::fs::write(&env, "NOOB_BASE_URL=http://elsewhere:9999/v1
").expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, AGENT);

        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Card(card) if card.does == Some(Doing::DefaultEndpoint)))
            .expect("the restore is on the section");
        let Some(Row::Card(card)) = panel.row(at) else {
            panic!("no card at {at}");
        };
        assert_eq!(card.does.expect("an action").word(), "default");
        assert!(
            card.hint.as_deref().unwrap_or_default().contains(agent::ENDPOINT_DEFAULT),
            "the card does not say what it writes: {:?}",
            card.hint
        );
        assert_eq!(agent::ENDPOINT_DEFAULT, "http://localhost:8080/v1");
        // It stands under the connection card, which is the field it writes.
        assert!(matches!(panel.row(at - 1), Some(Row::Card(card)) if card.title == "CONNECTION"));

        // The write itself is main's, through the same writer the field goes
        // through, and what comes back is the file.
        write_endpoint(&env, agent::ENDPOINT, agent::ENDPOINT_DEFAULT).expect("the file takes it");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, AGENT);
        assert!(said(&panel).contains(agent::ENDPOINT_DEFAULT), "{}", said(&panel));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every key of the agent's file is on a card, the one this window has no
    /// control for included.
    ///
    /// A key the window has never heard of is still a key the agent reads, so
    /// it is on the last card rather than dropped.
    #[test]
    fn the_agent_cards_keep_every_key() {
        let dir = scratch_dir("agent-form-order");
        std::fs::write(
            dir.join(".env"),
            "NOOB_BASE_URL=http://localhost:8080/v1\nNOOB_CTX=262144\nNOOB_API_KEY=sk-secret\nNOOB_MODEL=laguna-s\nNOOB_TIMEOUT=90\n",
        )
        .expect("a file");
        let mut panel = Settings::open(
            &Config::default(),
            Some(Path::new("/tmp/no0b.conf")),
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, AGENT);
        let rows = panel.rows().to_vec();
        // Every key in the file: the four with a field of their own by the name
        // and the sentence that field carries, and the one nothing here knows
        // about by its own key, on the card that holds the rest of the file.
        for key in ["NOOB_API_KEY", "NOOB_MODEL", "NOOB_TIMEOUT"] {
            assert!(
                rows.iter().any(|row| says(row).contains(key)),
                "{key} is not on the section: {rows:?}"
            );
        }
        let rest = rows
            .iter()
            .find_map(|row| match row {
                Row::Card(card) if card.title == "THE REST OF THE FILE" => Some(card),
                _ => None,
            })
            .expect("the card that carries what this window has no control for");
        assert_eq!(
            rest.fields
                .iter()
                .map(|field| field.label.as_str())
                .collect::<Vec<_>>(),
            ["NOOB_TIMEOUT"],
            "a key with a field of its own is listed twice"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

}
