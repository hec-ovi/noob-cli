//! Parsing a typed `/command` and resolving it to an act.
//!
//! One entry point: [`dispatch`] takes the typed line and the agent snapshot
//! (skills and servers are named by what is really on the disk) and answers
//! with an act for `main` to route, lines to say, or the refusal. A bad call
//! answers with the command's own usage line and nothing half-applied; an
//! unknown command answers with the nearest name.

use crate::agent::Agent;
use crate::config;
use crate::install;
use crate::settings::{self, Change, Deed, File, Kind};

use super::{manual, usage, Command, Does, Shape, ALL};

/// Whether a submitted line is for the window rather than for the agent.
pub fn is_command(text: &str) -> bool {
    text.trim_start().starts_with('/')
}

/// What a command asks `main` to do, and the line to say once it has landed.
pub enum Act {
    /// Write one setting, through the same writers a nudge on the panel goes
    /// through: the window file or the agent's .env, by `change.file`.
    Change { change: Change, said: String },
    /// One of the panel's deeds, done without the panel.
    Deed { deed: Deed, said: String },
    /// Install the source, already validated: the same background clone the
    /// panel's install button starts.
    Install { source: String, said: String },
    /// Open the settings panel: on one section by its rail index, or where
    /// it was left.
    Open {
        section: Option<usize>,
        said: String,
    },
}

/// What a typed line came to.
pub enum Answer {
    /// Something for `main` to do.
    Do(Act),
    /// Lines for the transcript: `/help`, or a call that needed nothing done.
    Say(Vec<String>),
    /// Why nothing happened: a bad argument with the command's usage line,
    /// or an unknown name with the nearest real one.
    Refuse(String),
}

/// Parse one typed line against the registry and resolve it.
pub fn dispatch(text: &str, agent: &Agent) -> Answer {
    let line = text.trim();
    let Some(line) = line.strip_prefix('/') else {
        return Answer::Refuse(String::from("a command starts with /; /help lists them"));
    };
    let (name, after) = match line.split_once(char::is_whitespace) {
        Some((name, after)) => (name, after.trim()),
        None => (line.trim(), ""),
    };
    if name.is_empty() {
        return Answer::Refuse(String::from("type a command after the /; /help lists them"));
    }
    let Some(command) = ALL.iter().find(|command| command.name == name) else {
        return Answer::Refuse(format!(
            "there is no /{name}; the nearest is /{}. /help lists them all",
            nearest(name)
        ));
    };
    let values = match take_args(command, after) {
        Ok(values) => values,
        Err(why) => return Answer::Refuse(format!("{why}; {}", usage(command))),
    };
    resolve(command, values, agent)
}

/// Split what was typed after the name into the command's arguments: one
/// token per `Word`, the rest of the line for `Rest`, nothing left over.
fn take_args(command: &Command, after: &str) -> Result<Vec<String>, String> {
    let mut left = after;
    let mut values = Vec::new();
    for arg in command.args {
        if matches!(arg.shape, Shape::Rest) {
            if left.is_empty() {
                match arg.optional {
                    true => return Ok(values),
                    false => return Err(format!("it needs <{}>", arg.name)),
                }
            }
            values.push(left.to_string());
            left = "";
            continue;
        }
        let (token, rest) = match left.split_once(char::is_whitespace) {
            Some((token, rest)) => (token, rest.trim()),
            None => (left, ""),
        };
        if token.is_empty() {
            match arg.optional {
                true => return Ok(values),
                false => return Err(format!("it needs <{}>", arg.name)),
            }
        }
        values.push(token.to_string());
        left = rest;
    }
    match left.is_empty() {
        true => Ok(values),
        false => Err(format!("{left:?} is more than it takes")),
    }
}

/// Turn a parsed call into its act, validating each argument against the
/// same definitions the panel offers: the tables' bounds, the theme list,
/// the palette's keys, the section names, and what the snapshot says is
/// installed or configured.
fn resolve(command: &Command, values: Vec<String>, agent: &Agent) -> Answer {
    match &command.does {
        Does::Set { key, file } => {
            let value = match spell_value(command, &values[0]) {
                Ok(value) => value,
                Err(why) => return Answer::Refuse(format!("{why}; {}", usage(command))),
            };
            let said = match file {
                File::Window => format!("{key} is {value}"),
                File::Agent => format!("{key} is {value}; the CLI reads it on its next request"),
            };
            Answer::Do(Act::Change {
                change: Change {
                    key,
                    value,
                    file: *file,
                },
                said,
            })
        }
        Does::SetColour => {
            let Some(key) = config::colour_keys()
                .into_iter()
                .find(|key| *key == values[0])
            else {
                return Answer::Refuse(format!(
                    "{} is not a colour key; press a swatch on the APPEARANCE section to see its key. {}",
                    values[0],
                    usage(command)
                ));
            };
            let value = values[1].clone();
            // The config parser is the one authority on what a colour is,
            // the same check a pressed swatch's Enter runs.
            if !colour_reads(key, &value) {
                return Answer::Refuse(format!(
                    "{value:?} is not a colour; {}",
                    usage(command)
                ));
            }
            Answer::Do(Act::Change {
                said: format!("{key} is {value}"),
                change: Change {
                    key,
                    value,
                    file: File::Window,
                },
            })
        }
        Does::TurnSkill { on } => {
            let Some(skill) = skill_named(agent, &values[0]) else {
                return Answer::Refuse(no_skill(agent, &values[0]));
            };
            let word = match on {
                true => "on",
                false => "off",
            };
            if skill.on == *on {
                return Answer::Say(vec![format!("the {} skill is already {word}", skill.name)]);
            }
            Answer::Do(Act::Deed {
                deed: Deed::TurnSkill {
                    dir: skill.dir.clone(),
                    on: *on,
                },
                said: format!(
                    "the {} skill is {word}; the agent picks that up on its next session",
                    skill.name
                ),
            })
        }
        Does::RemoveSkill => {
            let Some(skill) = skill_named(agent, &values[0]) else {
                return Answer::Refuse(no_skill(agent, &values[0]));
            };
            Answer::Do(Act::Deed {
                deed: Deed::RemoveSkill {
                    dir: skill.dir.clone(),
                    on: skill.on,
                },
                said: format!("the {} skill is removed: its directory is gone", skill.name),
            })
        }
        Does::InstallSkill => {
            let source = values[0].clone();
            // The validate rule: only a source the checker approves is
            // installed, the same gate the panel's button keeps.
            match install::check(&source) {
                Ok(said) => Answer::Do(Act::Install {
                    source,
                    said: format!("{said}\u{2026}"),
                }),
                Err(why) => Answer::Refuse(why),
            }
        }
        Does::TurnServer { on } => {
            let Some(server) = server_named(agent, &values[0]) else {
                return Answer::Refuse(no_server(agent, &values[0]));
            };
            let word = match on {
                true => "on",
                false => "off",
            };
            if server.on == *on {
                return Answer::Say(vec![format!("the {} server is already {word}", server.name)]);
            }
            Answer::Do(Act::Deed {
                deed: Deed::TurnServer {
                    name: server.name.clone(),
                    project: server.project,
                    on: *on,
                },
                said: format!("the {} server is {word} in its own file", server.name),
            })
        }
        Does::RemoveServer => {
            let Some(server) = server_named(agent, &values[0]) else {
                return Answer::Refuse(no_server(agent, &values[0]));
            };
            Answer::Do(Act::Deed {
                deed: Deed::RemoveServer {
                    name: server.name.clone(),
                    project: server.project,
                },
                said: format!("the {} server is out of its mcp.json", server.name),
            })
        }
        Does::AddServer => Answer::Do(Act::Deed {
            said: format!(
                "the {} server is in the global mcp.json; the agent connects on its next session",
                values[0]
            ),
            deed: Deed::AddServer {
                name: values[0].clone(),
                how: values[1].clone(),
            },
        }),
        Does::ForgetSession => Answer::Do(Act::Deed {
            said: format!("the {} conversation is deleted", values[0]),
            deed: Deed::ForgetSessions {
                ids: vec![values[0].clone()],
            },
        }),
        Does::RestoreLooks => Answer::Do(Act::Deed {
            deed: Deed::RestoreLooks,
            said: String::from(
                "every size, transparency and colour is back to what the window ships with",
            ),
        }),
        Does::Open { section } => Answer::Do(open(section)),
        Does::OpenPanel => match values.first() {
            None => Answer::Do(Act::Open {
                section: None,
                said: String::from("the settings panel is open"),
            }),
            Some(word) => match settings::SECTIONS
                .into_iter()
                .find(|name| section_word(name) == *word)
            {
                Some(name) => Answer::Do(open(name)),
                None => Answer::Refuse(format!(
                    "{word} is not a section; {}",
                    usage(command)
                )),
            },
        },
        Does::Help => match values.first() {
            None => Answer::Say(listing()),
            Some(name) => {
                let name = name.strip_prefix('/').unwrap_or(name);
                match ALL.iter().find(|command| command.name == name) {
                    Some(command) => Answer::Say(manual(command)),
                    None => Answer::Refuse(format!(
                        "there is no /{name}; the nearest is /{}. /help lists them all",
                        nearest(name)
                    )),
                }
            }
        },
    }
}

/// Opening one section of the panel, by the index the rail addresses it with.
fn open(section: &'static str) -> Act {
    let at = settings::SECTIONS
        .iter()
        .position(|name| *name == section)
        .expect("a command can only open a section on the rail");
    Act::Open {
        section: Some(at),
        said: format!("the settings panel is open on {}", settings::section_title(section)),
    }
}

/// A setting's typed value, checked and spelled the way the panel writes it:
/// a choice must be on the list, a number inside the same bounds the slider
/// moves in, written with the file's own decimals.
fn spell_value(command: &Command, typed: &str) -> Result<String, String> {
    let Some(kind) = kind_of(command) else {
        // The endpoint: typed text the panel does not bound either.
        return Ok(typed.to_string());
    };
    match kind {
        Kind::Choice(names) => match names.contains(&typed) {
            true => Ok(typed.to_string()),
            false => Err(format!("{typed} is not one of them")),
        },
        Kind::Number {
            low, high, places, ..
        } => {
            let value: f32 = typed
                .parse()
                .map_err(|_| format!("{typed:?} is not a number"))?;
            if !(low..=high).contains(&value) {
                return Err(format!(
                    "{typed} is outside {} to {}",
                    spell(low, places),
                    spell(high, places)
                ));
            }
            Ok(spell(value, places))
        }
    }
}

/// The bounds a Set command's value lives in, off the panel's own tables.
/// Nothing for the endpoint, which the panel types free too.
fn kind_of(command: &Command) -> Option<Kind> {
    let Does::Set { key, file } = &command.does else {
        return None;
    };
    let table: &[(&str, Kind)] = match file {
        File::Window => &settings::LOOKS,
        File::Agent => &settings::AGENT_SETTINGS,
    };
    table
        .iter()
        .find(|(known, _)| known == key)
        .map(|(_, kind)| *kind)
}

/// A number spelled the way the file spells it.
fn spell(value: f32, places: usize) -> String {
    format!("{value:.places$}")
}

/// Whether the config parser takes `key = value`, which is the same question
/// a pressed swatch's Enter asks before anything is written.
fn colour_reads(key: &str, value: &str) -> bool {
    config::Config::parse(&format!("{key} = {value}"))
        .unknown
        .is_empty()
}

/// The note after a usage line's arguments: what the one bounded or listed
/// argument takes. Nothing when every argument is free text.
pub(super) fn takes(command: &Command) -> Option<String> {
    if let Some(kind) = kind_of(command) {
        return Some(match kind {
            Kind::Choice(names) => format!("one of {}", names.join(", ")),
            Kind::Number {
                low, high, places, ..
            } => format!("{} to {}", spell(low, places), spell(high, places)),
        });
    }
    command.args.iter().find_map(|arg| match arg.shape {
        Shape::Colour => Some(String::from("the value as #rrggbb")),
        Shape::Section => Some(format!(
            "one of {}",
            settings::SECTIONS
                .iter()
                .map(|name| section_word(name))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => None,
    })
}

/// A section's rail word as it is typed: lower case, underscores for spaces.
pub fn section_word(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
}

/// What `/help` prints: every command with its line, in registry order.
fn listing() -> Vec<String> {
    let wide = ALL
        .iter()
        .map(|command| command.name.len())
        .max()
        .unwrap_or(0)
        + 1;
    let mut lines = vec![String::from(
        "the commands; /help <name> tells the whole story of one:",
    )];
    lines.push(String::new());
    for command in &ALL {
        lines.push(format!("/{:<wide$} {}", command.name, command.about));
    }
    lines
}

/// The registered name closest to what was typed.
fn nearest(typed: &str) -> &'static str {
    ALL.iter()
        .min_by_key(|command| distance(typed, command.name))
        .map(|command| command.name)
        .expect("the registry is never empty")
}

/// Plain edit distance, small words against a small list.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (row, ca) in a.chars().enumerate() {
        let mut next = vec![row + 1];
        for (at, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            next.push((prev[at] + cost).min(prev[at + 1] + 1).min(next[at] + 1));
        }
        prev = next;
    }
    prev[b.len()]
}

/// A skill by the name on its row or the directory it lives in, which are
/// the two ways the panel names one.
fn skill_named<'a>(agent: &'a Agent, name: &str) -> Option<&'a crate::agent::Skill> {
    agent
        .skills
        .iter()
        .find(|skill| skill.name == name || skill.dir == name)
}

fn no_skill(agent: &Agent, name: &str) -> String {
    let installed: Vec<&str> = agent.skills.iter().map(|skill| skill.name.as_str()).collect();
    match installed.is_empty() {
        true => format!("there is no {name} skill: nothing is installed. /skill_install adds one"),
        false => format!(
            "there is no {name} skill; installed: {}",
            installed.join(", ")
        ),
    }
}

/// A server by the name its entry carries.
fn server_named<'a>(agent: &'a Agent, name: &str) -> Option<&'a crate::agent::Server> {
    agent.mcp.servers.iter().find(|server| server.name == name)
}

fn no_server(agent: &Agent, name: &str) -> String {
    let configured: Vec<&str> = agent
        .mcp
        .servers
        .iter()
        .map(|server| server.name.as_str())
        .collect();
    match configured.is_empty() {
        true => format!("there is no {name} server: none are configured. /mcp_add writes one"),
        false => format!(
            "there is no {name} server; configured: {}",
            configured.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Server, Skill};
    use crate::config::Config;
    use std::path::PathBuf;

    /// An agent snapshot with one skill off and one server on, which is what
    /// the resolving commands name things against.
    fn snapshot() -> Agent {
        let mut agent = Agent::default();
        agent.skills.push(Skill {
            dir: "web-search".into(),
            name: "web-search".into(),
            about: String::new(),
            repo: None,
            path: PathBuf::from("/tmp/skills/web-search"),
            on: false,
            doc: Vec::new(),
        });
        agent.mcp.servers.push(Server {
            name: "context7".into(),
            how: "https://mcp.context7.com/mcp".into(),
            project: true,
            on: true,
            entry: String::new(),
        });
        agent
    }

    fn refused(text: &str) -> String {
        match dispatch(text, &snapshot()) {
            Answer::Refuse(why) => why,
            _ => panic!("{text} was not refused"),
        }
    }

    fn acted(text: &str) -> Act {
        match dispatch(text, &snapshot()) {
            Answer::Do(act) => act,
            Answer::Refuse(why) => panic!("{text} was refused: {why}"),
            Answer::Say(said) => panic!("{text} only said {said:?}"),
        }
    }

    /// The paths through the parser: a good call resolves, a bad argument
    /// answers with the command's own usage line and nothing else happens,
    /// an unknown name answers with the nearest real one, and a bare slash
    /// is told where the list is.
    #[test]
    fn parsing_good_bad_and_unknown() {
        let Act::Change { change, said } = acted("/set_theme noob-red") else {
            panic!("a theme set is a change");
        };
        assert_eq!(change.key, "theme");
        assert_eq!(change.value, "noob-red");
        assert_eq!(change.file, File::Window);
        assert!(said.contains("noob-red"), "{said}");

        // Out of the list, out of bounds, not a number, missing, extra:
        // every one answers with the usage line and half-applies nothing.
        for bad in [
            "/set_theme neon",
            "/set_prompt_rows 99",
            "/set_prompt_rows tall",
            "/set_prompt_rows",
            "/restore_appearance now",
        ] {
            let why = refused(bad);
            assert!(why.contains("usage: /"), "{bad}: {why}");
        }

        let why = refused("/set_thene noob-red");
        assert!(why.contains("/set_theme"), "{why}");
        assert!(why.contains("/help"), "{why}");
        assert!(refused("/").contains("/help"));

        // A number is spelled the way the file spells it, inside the
        // panel's own bounds.
        let Act::Change { change, .. } = acted("/set_transparency_window 0.5") else {
            panic!("a transparency set is a change");
        };
        assert_eq!(change.value, "0.50");
        let Act::Change { change, .. } = acted("/set_context_window 262144") else {
            panic!("a context set is a change");
        };
        assert_eq!(change.key, crate::agent::CTX);
        assert_eq!(change.file, File::Agent);
    }

    /// The commands that name things on the disk resolve them off the
    /// snapshot: a skill by name or directory, a server with the file it
    /// really sits in, and a name that is not there is refused with what is.
    #[test]
    fn names_resolve_off_the_snapshot() {
        let Act::Deed { deed, .. } = acted("/skill_on web-search") else {
            panic!("a skill turn is a deed");
        };
        assert_eq!(
            deed,
            Deed::TurnSkill {
                dir: String::from("web-search"),
                on: true
            }
        );
        // Already in the asked-for state: something to say, nothing to do.
        match dispatch("/skill_off web-search", &snapshot()) {
            Answer::Say(said) => assert!(said[0].contains("already off"), "{said:?}"),
            _ => panic!("an already-off skill is only said"),
        }
        let Act::Deed { deed, .. } = acted("/mcp_off context7") else {
            panic!("a server turn is a deed");
        };
        assert_eq!(
            deed,
            Deed::TurnServer {
                name: String::from("context7"),
                project: true,
                on: false
            }
        );
        let Act::Deed { deed, .. } = acted("/mcp_add graphiti uvx graphiti-mcp") else {
            panic!("an add is a deed");
        };
        assert_eq!(
            deed,
            Deed::AddServer {
                name: String::from("graphiti"),
                how: String::from("uvx graphiti-mcp")
            }
        );
        let Act::Deed { deed, .. } = acted("/delete_session 2026-07-30-abcd") else {
            panic!("a session delete is a deed");
        };
        assert_eq!(
            deed,
            Deed::ForgetSessions {
                ids: vec![String::from("2026-07-30-abcd")]
            }
        );
        assert!(refused("/skill_on nothing-here").contains("web-search"));
        assert!(refused("/mcp_on nothing-here").contains("context7"));
    }

    /// The validate rule: /skill_install answers a source the checker
    /// refuses with the checker's own reason, and a repository resolves to
    /// the background install with the checker's verdict said first.
    #[test]
    fn installs_are_validated_first() {
        let why = refused("/skill_install not a source at all");
        assert!(!why.is_empty());
        let Act::Install { source, said } = acted("/skill_install hec-ovi/websearch-skill") else {
            panic!("a good source installs");
        };
        assert_eq!(source, "hec-ovi/websearch-skill");
        assert!(said.contains("repository"), "{said}");
    }

    /// /help lists every registered command with its line; /help <name>
    /// prints that command's usage and manual, slash or no slash.
    #[test]
    fn help_tells_the_whole_registry() {
        let Answer::Say(lines) = dispatch("/help", &snapshot()) else {
            panic!("/help says");
        };
        let text = lines.join("\n");
        for command in &ALL {
            assert!(text.contains(&format!("/{}", command.name)), "{}", command.name);
            assert!(text.contains(command.about), "{}", command.about);
        }
        let Answer::Say(lines) = dispatch("/help /set_theme", &snapshot()) else {
            panic!("/help <name> says");
        };
        let theme = ALL.iter().find(|c| c.name == "set_theme").unwrap();
        assert_eq!(lines, manual(theme));
        assert!(lines[0].contains("noob-matrix"), "the usage names the themes: {lines:?}");
    }

    /// Every section on the rail is reachable by its typed word, and the
    /// openers land on the section they name. Walked off SECTIONS itself,
    /// so a section added to the rail without a typed word fails here.
    #[test]
    fn every_section_opens_by_its_word() {
        for (at, name) in settings::SECTIONS.iter().enumerate() {
            let text = format!("/settings {}", section_word(name));
            let Act::Open { section, .. } = acted(&text) else {
                panic!("{text} opens");
            };
            assert_eq!(section, Some(at), "{text}");
        }
        let Act::Open { section, .. } = acted("/settings") else {
            panic!("/settings opens");
        };
        assert_eq!(section, None);
        let Act::Open { section, said } = acted("/show_system_prompt") else {
            panic!("the prompt opener opens");
        };
        assert_eq!(
            settings::SECTIONS[section.unwrap()],
            settings::PROMPT
        );
        assert!(said.contains("SYSTEM PROMPT"), "{said}");
        let Act::Open { section, .. } = acted("/sessions") else {
            panic!("the sessions opener opens");
        };
        assert_eq!(settings::SECTIONS[section.unwrap()], settings::SESSIONS);
    }

    /// Every theme and every colour key the config owns is typable, walked
    /// off the config's own lists; and what the two writes produce lands in
    /// a real file through the same commit the panel uses.
    #[test]
    fn themes_and_colours_land_in_the_file() {
        for theme in config::THEMES {
            let Act::Change { change, .. } = acted(&format!("/set_theme {theme}")) else {
                panic!("{theme} sets");
            };
            assert_eq!(change.value, theme);
        }
        for key in config::colour_keys() {
            let Act::Change { change, .. } = acted(&format!("/set_color {key} #123456")) else {
                panic!("{key} sets");
            };
            assert_eq!(change.key, key);
        }
        assert!(refused("/set_color accent red").contains("usage: /set_color"));
        assert!(refused("/set_color nothing #123456").contains("not a colour key"));

        // The same two writes, through the real file: the theme line lands
        // and the palette follows it; the transparency reads back.
        let path = settings::testing::scratch("commands-theme");
        std::fs::write(&path, "").expect("a scratch settings file");
        let Act::Change { change, .. } = acted("/set_theme noob-red") else {
            panic!("a theme set is a change");
        };
        let config = settings::commit(&path, &change).expect("the theme lands");
        assert_ne!(
            config.accent,
            Config::default().accent,
            "the palette follows the theme"
        );
        let Act::Change { change, .. } = acted("/set_transparency_panels 0.35") else {
            panic!("a transparency set is a change");
        };
        let config = settings::commit(&path, &change).expect("the opacity lands");
        assert_eq!(config.opacity, 0.35);
        let written = std::fs::read_to_string(&path).expect("the file is there");
        assert!(written.contains("theme = noob-red"), "{written}");
        assert!(written.contains("opacity = 0.35"), "{written}");
    }

    /// The usage lines say what an argument takes: the themes are named, the
    /// bounds are the panel's own, and the sections are typable words.
    #[test]
    fn usage_says_what_an_argument_takes() {
        let theme = ALL.iter().find(|c| c.name == "set_theme").unwrap();
        assert!(usage(theme).contains("noob-matrix, noob-cool, noob-red"));
        let rows = ALL.iter().find(|c| c.name == "set_prompt_rows").unwrap();
        assert!(usage(rows).contains("1 to 24"), "{}", usage(rows));
        let panel = ALL.iter().find(|c| c.name == "settings").unwrap();
        assert!(usage(panel).contains("system_prompt"), "{}", usage(panel));
    }
}
