//! The AGENT section: what the CLI is pointed at, out of the file the CLI
//! owns, and the whole assembled prompt under it.
//!
//! One of the settings panel's nested section boxes. It builds rows out of the
//! shared vocabulary in [`crate::settings`] and owns the plain-words names for
//! the agent's env keys; the frame owns the cursor, the writes, and the
//! [`Assembled`] prompt state it hands in here.

use crate::agent::{self, Agent};
use crate::settings::{Assembled, Card, CardField, File, Kind, Paper, Row, SECRET, UNSET};

/// The keys of the agent's file this section draws a field of its own for: what
/// to call each one in plain words, and the sentence under it.
///
/// The key is in the sentence rather than in the label. `NOOB_TASK_CONCURRENCY`
/// over a number says nothing to somebody who has not read the CLI, and the
/// whole complaint about this section was that half its rows said nothing about
/// what they did; the sentence is also the answer to "which line do I edit",
/// since three of these five are only editable in the file.
///
/// Every sentence here is read off the CLI: the bounds and the fallbacks are in
/// [`crate::agent`], the two request shapes and the thinking switch are what
/// `noob-provider` refuses anything else for, and the probe is what a missing
/// base URL really does.
const AGENT_FIELDS: [(&str, &str, &str); 7] = [
    (
        agent::ENDPOINT,
        "endpoint",
        "NOOB_BASE_URL. Left empty, noob probes the usual local ports",
    ),
    (
        agent::API_KEY,
        "api key",
        "NOOB_API_KEY. Sent as the bearer token, and never shown here",
    ),
    (
        agent::MODEL,
        "model",
        "NOOB_MODEL. Which model to ask that endpoint for, by its name",
    ),
    (
        agent::API_STYLE,
        "api style",
        "NOOB_API_STYLE. chat or responses; unset, noob picks by the address",
    ),
    (
        agent::REASONING,
        "reasoning",
        "NOOB_REASONING. on or off; unset, the server's own flags decide",
    ),
    (
        agent::CTX,
        "context window",
        "NOOB_CTX. Tokens a session is budgeted before the CLI compacts it",
    ),
    (
        agent::TASK_CONCURRENCY,
        "max sub-agents",
        "NOOB_TASK_CONCURRENCY. How many sub-agent tasks may run at once, capped at sixteen",
    ),
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
pub(crate) const AGENT_SETTINGS: [(&str, Kind); 2] = [
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
        agent::TASK_CONCURRENCY,
        Kind::Number {
            step: agent::TASK_CONCURRENCY_STEP,
            low: agent::TASK_CONCURRENCY_LOW,
            high: agent::TASK_CONCURRENCY_HIGH,
            places: 0,
            stops: &agent::TASK_CONCURRENCY_STOPS,
        },
    ),
];

/// What the CLI uses for one of its own settings when the file does not carry
/// it. Read off the CLI rather than chosen here: a row that shows a number the
/// agent is not actually running with is worse than no row.
fn agent_default(key: &str) -> String {
    match key {
        agent::CTX => agent::CTX_DEFAULT.to_string(),
        agent::TASK_CONCURRENCY => agent::TASK_CONCURRENCY_DEFAULT.to_string(),
        // Unreachable through AGENT_SETTINGS, and a number is the honest answer
        // for a row that says it is one.
        _ => String::from("0"),
    }
}

/// What the agent is pointed at, out of the file the CLI owns.
///
/// Cards, in the order somebody meeting this window needs them: where the
/// model is, which model it is, how much the agent gets, the file all of it
/// is written in, and whatever else that file carries. Then the whole
/// assembled prompt, a card as well.
///
/// "actually is awful as is now, unclear because has too many lines
/// between". It was a two column form of raw environment keys with three
/// notes standing in the middle of it, and half its rows said nothing about
/// what they did: `NOOB_TASK_CONCURRENCY 4` is not a setting anybody can
/// act on. Every field is a plain-words label over its value now, with the
/// key and what it decides in one sentence under it, and the space between
/// them is the card rather than a line.
pub fn rows(agent: &Agent, prompt: &Assembled) -> Vec<Row> {
    let mut unset = Vec::new();
    let mut numbers = Vec::new();
    for (key, kind) in AGENT_SETTINGS {
        let value = match agent.setting(key) {
            Some(value) => value.to_string(),
            None => {
                unset.push(key);
                agent_default(key)
            }
        };
        let (label, hint) = agent_says(key);
        numbers.push(CardField::setting(label, key, value, kind, File::Agent).saying(hint));
    }
    let tasks = numbers.pop().expect("both of the agent's numbers");
    let ctx = numbers.pop().expect("both of the agent's numbers");
    let mut rows = vec![
        // The endpoint first, because an agent pointed at nothing is the
        // one state this window cannot work in, and its credential beside
        // it, because that is the other half of reaching a server.
        Row::Card(Card {
        does: None,
            title: String::from("CONNECTION"),
            fields: vec![
                CardField::text(
                    "endpoint",
                    agent::ENDPOINT,
                    agent.endpoint().unwrap_or_default().to_string(),
                )
                .saying(agent_says(agent::ENDPOINT).1),
                CardField::reading("api key", env_says(agent, agent::API_KEY))
                    .saying(agent_says(agent::API_KEY).1),
            ],
            hint: None,
        }),
        Row::Card(Card {
        does: None,
            title: String::from("MODEL"),
            fields: [agent::MODEL, agent::API_STYLE, agent::REASONING]
                .into_iter()
                .map(|key| {
                    let (label, hint) = agent_says(key);
                    CardField::reading(label, env_says(agent, key)).saying(hint)
                })
                .collect(),
            // Said once, on the card, rather than three times under three
            // fields that cannot be typed into.
            hint: Some(String::from(
                "read from the settings file; edit them there, or export the variable",
            )),
        }),
        Row::Card(Card {
        does: None,
            title: String::from("LIMITS"),
            fields: vec![ctx, tasks],
            hint: match unset.is_empty() {
                true => None,
                // A slider showing a number nobody wrote, with nothing
                // saying so, is a window inventing a setting.
                false => Some(format!(
                    "{} not set: showing the built-in default; changing it writes the line",
                    unset.join(" and ")
                )),
            },
        }),
        Row::Card(Card {
        does: None,
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
        .map(|(key, _)| {
            // A credential is reported as set and never as itself. The CLI
            // keeps secrets out of settable config on purpose, and a window
            // is a worse place for one than a terminal: it is on a screen
            // somebody else can be standing behind.
            CardField::reading(key, env_says(agent, key))
        })
        .collect();
    if !rest.is_empty() {
        rows.push(Row::Card(Card {
        does: None,
            title: String::from("THE REST OF THE FILE"),
            fields: rest,
            hint: Some(String::from(
                "keys the CLI reads that this window has no control for: edit them in the file",
            )),
        }));
    }
    // The prompt last, and nothing under it: it is a screenful, and a field
    // below it is a field nobody scrolls to.
    rows.push(Row::Paper(prompt_paper(prompt)));
    rows
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

/// The whole prompt, exactly as the CLI assembles it.
///
/// The global `AGENTS.md` (the SYSTEM PROMPT section's own document) is one
/// layer of this: the prompt also carries the CLI's own base instructions,
/// the environment block, the project's own AGENTS.md, the skills resolver
/// and the MCP line. Only `noob debug prompt` returns all of it, so that is
/// what this block shows, and while it is running or after it has failed the
/// block says which of the two happened.
fn prompt_paper(prompt: &Assembled) -> Paper {
    let title = String::from("THE PROMPT THE AGENT GETS");
    match prompt {
        Assembled::Waiting => Paper {
            title,
            under: String::from("running noob debug prompt\u{2026}"),
            body: Vec::new(),
            first: 0,
            offer: None,
            bad: false,
        },
        Assembled::Got { at, body } => Paper {
            title,
            under: format!("noob debug prompt, run in {at}"),
            body: body.clone(),
            first: 0,
            offer: None,
            bad: false,
        },
        Assembled::Failed { at, why } => Paper {
            title,
            under: format!("{why} (run in {at})"),
            body: Vec::new(),
            first: 0,
            offer: None,
            bad: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::settings::testing::*;
    use crate::settings::{
        card_is_reachable, commit, landable, lines, paper_body_lines, write_endpoint, Change,
        Settings, Side, AGENT, PAPER_LINES,
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
        // The number is under a name anybody can read, with the key that writes
        // it in the sentence under it rather than standing in for the name.
        assert!(text.contains("context window 262144"), "{text}");
        assert!(text.contains("NOOB_CTX."), "{text}");
        // The concurrency row is named for what it caps, not for a phrase
        // nobody could map to the key.
        assert!(text.contains("max sub-agents"), "{text}");
        assert!(text.contains("sub-agent tasks may run at once"), "{text}");
        assert!(!text.contains("sk-secret"), "a credential is on the panel: {text}");
        assert!(text.contains(&format!("api key {SECRET}")), "{text}");
        assert_eq!(panel.agent_file(), Some(dir.join(".env").as_path()));

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
        // bottom, and at the top the sixteen it caps itself at, so the maximum
        // is somewhere the pointer can be dropped rather than a number to guess.
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
                value: String::from("16"),
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
        assert_eq!(value(&panel, agent::TASK_CONCURRENCY), "16");
        let after = std::fs::read_to_string(&env).expect("the file");
        assert!(after.contains("NOOB_TASK_CONCURRENCY=16"), "{after}");
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

        // The concurrency track: near five means five, where the plain step
        // would have said four.
        put_cursor(&mut panel, agent::TASK_CONCURRENCY);
        let at = panel.cursor();
        assert!(panel.slide(at, panel.side(), 0.22));
        assert_eq!(panel.preview(at, panel.side()), Some("5"));
        panel.drop_slider();

        // And every checkpoint is a tick on its track, in track order.
        for (key, kind) in AGENT_SETTINGS {
            let ticks = kind.stop_fractions();
            let Kind::Number { stops, .. } = kind else {
                panic!("{key} is not a track");
            };
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
        assert!(text.contains("not set: showing the built-in default"), "{text}");
        assert!(text.contains(agent::CTX), "{text}");
        assert!(text.contains(agent::TASK_CONCURRENCY), "{text}");
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

        // Cards, in the order somebody needs them, and the prompt last.
        let titles: Vec<String> = panel
            .rows()
            .iter()
            .map(|row| match row {
                Row::Card(card) => card.title.clone(),
                Row::Paper(paper) => paper.title.clone(),
                other => panic!("the section carries a loose row: {other:?}"),
            })
            .collect();
        assert_eq!(
            titles,
            [
                "CONNECTION",
                "MODEL",
                "LIMITS",
                "THE SETTINGS FILE",
                "THE PROMPT THE AGENT GETS",
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
                assert!(
                    field.label.chars().all(|ch| !ch.is_ascii_uppercase()),
                    "{}: {} is a key rather than a name",
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
        // The api key beside it is read out, so there is nowhere to cross to and
        // the cursor stays where something can be done.
        assert!(!panel.cross(Side::Right), "the cursor crossed onto a reading");

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
            matches!(panel.at_cursor(), Some(Row::Setting { key, .. }) if *key == agent::TASK_CONCURRENCY)
        );

        // And the cards of readings hold no cursor at all: a row the arrow keys
        // stop on where no key does anything is a dead stop.
        for (at, row) in panel.rows().iter().enumerate() {
            let Row::Card(card) = row else {
                continue;
            };
            let settable = card.fields.iter().any(CardField::editable);
            assert_eq!(
                landable(row),
                settable,
                "row {at}, {}, holds the cursor over nothing",
                card.title
            );
        }

        // Up and down walk the cards that can be set and the prompt block, and
        // every one of them is reachable from the top.
        assert!(panel.jump(false));
        let mut seen = vec![panel.cursor()];
        while panel.step(true) {
            seen.push(panel.cursor());
        }
        assert_eq!(
            seen,
            vec![0, numbers, numbers + 2],
            "the keyboard cannot walk the section: {seen:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every key of the agent's file is on a card above the prompt block, the
    /// one this window has no control for included.
    ///
    /// The rest of the environment used to be pushed on after the block, which
    /// left it about thirty lines under the rows it reads with: the section went
    /// form, a document, and then more form. A key the window has never heard of
    /// is still a key the agent reads, so it is on the last card rather than
    /// dropped.
    #[test]
    fn the_agent_cards_keep_every_key_above_the_block() {
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
        let first_block = rows
            .iter()
            .position(|row| matches!(row, Row::Paper(_)))
            .unwrap_or_else(|| panic!("there is no block at all: {rows:?}"));
        assert!(
            matches!(&rows[first_block], Row::Paper(paper) if paper.title.contains("PROMPT")),
            "the block is not the prompt: {:?}",
            rows[first_block]
        );
        // Every key in the file: the four with a field of their own by the name
        // and the sentence that field carries, and the one nothing here knows
        // about by its own key, on the card that holds the rest of the file.
        for key in ["NOOB_API_KEY", "NOOB_MODEL", "NOOB_TIMEOUT"] {
            let at = rows
                .iter()
                .position(|row| says(row).contains(key))
                .unwrap_or_else(|| panic!("{key} is not on the section: {rows:?}"));
            assert!(
                at < first_block,
                "{key} is row {at}, under the block at {first_block}"
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
        assert!(
            rows[first_block..].iter().all(|row| matches!(row, Row::Paper(_))),
            "there is a card under the blocks: {:?}",
            &rows[first_block..]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The block under the form: the whole prompt the agent gets.
    ///
    /// The prompt carries the CLI's base instructions, the environment block,
    /// both AGENTS.md layers, the skills resolver and the MCP line, and only
    /// `noob debug prompt` returns all of it. The block is a fixed height and
    /// reads with the page keys, so a prompt a thousand lines long does not
    /// turn the section into a text file.
    #[test]
    fn the_agent_section_carries_the_whole_prompt() {
        let dir = scratch_dir("agent-prompt-block");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, AGENT);

        let block = |panel: &Settings, title: &str| -> Paper {
            panel
                .rows()
                .iter()
                .find_map(|row| match row {
                    Row::Paper(paper) if paper.title.contains(title) => Some(paper.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("there is no {title} block: {:?}", panel.rows()))
        };
        // Until the CLI answers, the prompt block says it is being read rather
        // than drawing an empty box.
        assert!(
            block(&panel, "PROMPT").under.contains("running"),
            "{}",
            block(&panel, "PROMPT").under
        );
        let body: Vec<String> = (0..PAPER_LINES * 3).map(|at| format!("line {at}")).collect();
        panel.adopt_prompt(
            String::from("/home/hec/workspace/noob-cli"),
            Ok(body.clone()),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        let whole = block(&panel, "PROMPT");
        assert_eq!(whole.body, body);
        assert!(whole.under.contains("/home/hec/workspace/noob-cli"), "{}", whole.under);

        // A block is the same height whatever is in it, which is what keeps the
        // rows under it where the clicks below them are tested for. It is a card
        // like every other row of the section: its title in the header, where
        // the text came from and the text itself in the body.
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Paper(paper) if paper.title.contains("PROMPT")))
            .expect("the prompt block");
        let tall = crate::design::card_row_lines(paper_body_lines(), false);
        assert!(tall > PAPER_LINES, "a block shows fewer lines than it holds");
        assert_eq!(lines(panel.row(at).expect("the row"), COLS), tall);
        assert_eq!(panel.heights(COLS)[at], tall, "the model and the window disagree");

        // And it is read with the page keys: the cursor is on it, the block
        // moves and the list under it does not.
        assert!(panel.point_at(at, Side::Left));
        assert!(panel.hint().contains("page"), "{}", panel.hint());
        let was = panel.first();
        assert!(panel.page(20, true));
        assert_eq!(panel.paper(at).expect("the block").first, PAPER_LINES);
        assert_eq!(panel.cursor(), at, "reading the block walked the list");
        assert_eq!(panel.first(), was, "reading the block scrolled the section");
        assert!(panel.page(20, false));
        assert_eq!(panel.paper(at).expect("the block").first, 0);
        assert!(!panel.page(20, false), "it scrolled past its own first line");

        // Home and End take it the whole way, which is the pair of keys every
        // other scrolling thing in this window answers. The wheel used to be the
        // only route to the end of a block.
        assert!(panel.jump(true));
        assert_eq!(
            panel.paper(at).expect("the block").first,
            body.len() - PAPER_LINES,
            "End did not reach the last screenful"
        );
        assert_eq!(panel.first(), was, "reading the block scrolled the section");
        assert!(!panel.jump(true), "it jumped past its own last screenful");
        assert!(panel.jump(false));
        assert_eq!(panel.paper(at).expect("the block").first, 0);
        assert!(!panel.jump(false), "it jumped past its own first line");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A prompt the CLI would not print says why instead of showing nothing.
    #[test]
    fn a_failed_prompt_says_why() {
        let dir = scratch_dir("agent-prompt-failed");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, AGENT);
        panel.adopt_prompt(
            String::from("/tmp/work"),
            Err(String::from("noob debug prompt failed: no such subcommand")),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        let prompt = panel
            .rows()
            .iter()
            .find_map(|row| match row {
                Row::Paper(paper) if paper.title.contains("PROMPT") => Some(paper),
                _ => None,
            })
            .expect("the prompt block");
        assert!(prompt.bad, "a failure is not marked as one");
        assert!(prompt.under.contains("no such subcommand"), "{}", prompt.under);
        assert!(prompt.under.contains("/tmp/work"), "{}", prompt.under);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
