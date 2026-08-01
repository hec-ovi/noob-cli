//! The slash commands: every capability the settings panel exposes, typed
//! into the prompt as `/command`, plus `/help` over all of them.
//!
//! The registry here is data. The COMMANDS section of the settings panel
//! lists it, `dispatch` parses a typed line against it, and what a command
//! does is said as the same operations the panel performs: a [`Change`] or a
//! [`Deed`] the frame already routes, a skill install, or opening the panel
//! on a section. Nothing in this box touches a disk; `main` routes the acts
//! exactly where the panel's own presses land.
//!
//! Completeness is enforced against the sections' own definitions rather
//! than by a list kept by hand: the tests walk the settings tables, the
//! theme names, the colour keys, the section list and the panel's deed and
//! button vocabularies, and a capability without a command fails the build.

mod dispatch;

pub use dispatch::{dispatch, is_command, Act, Answer};

use crate::settings::File;

/// One command: its name after the slash, what it does in one line, the
/// longer help `/help <name>` prints, the arguments it takes, and what
/// running it means.
pub struct Command {
    /// The word after the slash, snake_case.
    pub name: &'static str,
    /// One line for the list and for `/help`.
    pub about: &'static str,
    /// The longer help, a line per entry.
    pub help: &'static [&'static str],
    pub args: &'static [Arg],
    pub(crate) does: Does,
}

/// One argument in the usage line, in order. Only the last may be `Rest` or
/// optional.
pub struct Arg {
    /// The word in the usage line: `/set_theme <theme>`.
    pub name: &'static str,
    pub(crate) shape: Shape,
    pub optional: bool,
}

impl Arg {
    const fn of(name: &'static str, shape: Shape) -> Arg {
        Arg {
            name,
            shape,
            optional: false,
        }
    }

    const fn maybe(name: &'static str, shape: Shape) -> Arg {
        Arg {
            name,
            shape,
            optional: true,
        }
    }
}

/// What an argument may hold. Validation happens in `dispatch`, against the
/// same tables the panel nudges inside, so a command cannot accept a value
/// the panel would refuse.
pub(crate) enum Shape {
    /// The value for the setting the command writes: its bounds and spelling
    /// come off the panel's own tables ([`crate::settings::LOOKS`] and
    /// [`crate::settings::AGENT_SETTINGS`]), looked up by the command's key.
    Value,
    /// One whitespace-free token.
    Word,
    /// Everything to the end of the line, trimmed. Never empty.
    Rest,
    /// A colour as `#rrggbb`, checked by the config parser itself.
    Colour,
    /// One of the palette's keys, out of [`crate::config::colour_keys`].
    ColourKey,
    /// A section of the settings panel, by its rail word in snake_case.
    Section,
}

/// What running a command means, said as the operations the panel performs.
pub(crate) enum Does {
    /// Write one setting, exactly as a nudge on the panel writes it. The key
    /// must be on the panel's own table for its file, which the completeness
    /// test proves.
    Set {
        key: &'static str,
        file: File,
    },
    /// Write one palette colour, the way a pressed swatch does.
    SetColour,
    /// Move a skill between on and off, by the name or directory typed.
    TurnSkill {
        on: bool,
    },
    /// Delete a skill's directory.
    RemoveSkill,
    /// Validate the typed source and, when it checks out, install it: the
    /// same two steps the panel's card walks.
    InstallSkill,
    /// Turn a configured MCP server on or off inside its own file.
    TurnServer {
        on: bool,
    },
    /// Take a server's entry out of its mcp.json.
    RemoveServer,
    /// Write a server into the global mcp.json.
    AddServer,
    /// Delete one saved conversation by its id.
    ForgetSession,
    /// Comment every appearance line out of the settings file.
    RestoreLooks,
    /// Open the settings panel on one section. The command for anything that
    /// only happens on the panel, and its help says so.
    Open {
        section: &'static str,
    },
    /// Open the settings panel, on a named section or where it was.
    OpenPanel,
    Help,
}

/// Every command, in the order the COMMANDS section lists them: the agent's
/// own settings first, then each section's acts, the panel itself, and help.
pub const ALL: [Command; 24] = [
    Command {
        name: "set_endpoint",
        about: "point the agent at a model server",
        help: &[
            "Writes NOOB_BASE_URL in the agent's .env, the same line the",
            "AGENT section's endpoint field writes. The CLI reads the file",
            "on its next request. Left unset, noob probes the usual local",
            "ports instead.",
        ],
        args: &[Arg::of("url", Shape::Rest)],
        does: Does::Set {
            key: crate::agent::ENDPOINT,
            file: File::Agent,
        },
    },
    Command {
        name: "set_context_window",
        about: "tokens a session is budgeted before the CLI compacts it",
        help: &[
            "Writes NOOB_CTX in the agent's .env, held to the same bounds",
            "the AGENT section's slider moves in. The CLI reads it on its",
            "next request.",
        ],
        args: &[Arg::of("tokens", Shape::Value)],
        does: Does::Set {
            key: crate::agent::CTX,
            file: File::Agent,
        },
    },
    Command {
        name: "set_max_subagents",
        about: "the most sub-agent tasks the CLI runs at once",
        help: &[
            "Writes NOOB_TASK_CONCURRENCY in the agent's .env, capped at",
            "sixteen the way the AGENT section's slider is. The CLI reads",
            "it on its next request.",
        ],
        args: &[Arg::of("count", Shape::Value)],
        does: Does::Set {
            key: crate::agent::TASK_CONCURRENCY,
            file: File::Agent,
        },
    },
    Command {
        name: "show_system_prompt",
        about: "open the prompt's three layers on the settings panel",
        help: &[
            "Opens the SYSTEM PROMPT section: AGENTS.md, TOOLS.md and the",
            "environment block, in the order the CLI assembles them.",
            "Editing happens on the panel, where the enable-edition",
            "checkbox opens a document and Ctrl-S writes its file; a",
            "command line is no place to type a document, so this one only",
            "takes you there.",
        ],
        args: &[],
        does: Does::Open {
            section: crate::settings::PROMPT,
        },
    },
    Command {
        name: "sessions",
        about: "open the saved conversations on the settings panel",
        help: &[
            "Opens the SESSIONS section: every saved conversation with its",
            "id, folder and size. Marking and deleting in bulk happens",
            "there; /delete_session removes one by id from here.",
        ],
        args: &[],
        does: Does::Open {
            section: crate::settings::SESSIONS,
        },
    },
    Command {
        name: "delete_session",
        about: "delete one saved conversation by its id",
        help: &[
            "Removes the transcript and the note about it, the same delete",
            "the SESSIONS table does to a marked row. The id is on the",
            "SESSIONS section (/sessions lists them). There is no undo.",
        ],
        args: &[Arg::of("id", Shape::Rest)],
        does: Does::ForgetSession,
    },
    Command {
        name: "skill_install",
        about: "validate a source and install it as a skill",
        help: &[
            "The same two steps the SKILLS card walks: the source is",
            "checked first (a git address, an owner/name shorthand, or a",
            "folder with a SKILL.md in it), and only a source that checks",
            "out is installed. The clone runs in the background and the",
            "answer lands here when it is done.",
        ],
        args: &[Arg::of("source", Shape::Rest)],
        does: Does::InstallSkill,
    },
    Command {
        name: "skill_on",
        about: "turn an installed skill on",
        help: &[
            "Moves the skill's directory back where the agent loads from,",
            "exactly what the toggle on its row does. Name it by the name",
            "on its row or by its directory.",
        ],
        args: &[Arg::of("name", Shape::Rest)],
        does: Does::TurnSkill { on: true },
    },
    Command {
        name: "skill_off",
        about: "turn an installed skill off",
        help: &[
            "Moves the skill's directory into the off sibling beside the",
            "skills directory, exactly what the toggle on its row does:",
            "still installed, not loaded.",
        ],
        args: &[Arg::of("name", Shape::Rest)],
        does: Does::TurnSkill { on: false },
    },
    Command {
        name: "skill_remove",
        about: "delete an installed skill's directory",
        help: &[
            "Deletes the skill's directory, the same delete the uninstall",
            "button does after its second press. There is no undo;",
            "/skill_install puts one back.",
        ],
        args: &[Arg::of("name", Shape::Rest)],
        does: Does::RemoveSkill,
    },
    Command {
        name: "mcp_add",
        about: "write an MCP server into the global mcp.json",
        help: &[
            "The same deed the ADD A SERVER card does: the entry goes into",
            "the global mcp.json under the name given, as a url when the",
            "second argument reads as one and as a command line otherwise.",
            "The agent connects on its next session.",
        ],
        args: &[
            Arg::of("name", Shape::Word),
            Arg::of("command-or-url", Shape::Rest),
        ],
        does: Does::AddServer,
    },
    Command {
        name: "mcp_on",
        about: "turn a configured MCP server on",
        help: &[
            "Moves the server's entry back under servers in its own file,",
            "exactly what the toggle on its row does.",
        ],
        args: &[Arg::of("name", Shape::Rest)],
        does: Does::TurnServer { on: true },
    },
    Command {
        name: "mcp_off",
        about: "turn a configured MCP server off",
        help: &[
            "Moves the server's entry out of servers in its own file,",
            "exactly what the toggle on its row does: still configured,",
            "not connected to.",
        ],
        args: &[Arg::of("name", Shape::Rest)],
        does: Does::TurnServer { on: false },
    },
    Command {
        name: "mcp_remove",
        about: "take a server's entry out of its mcp.json",
        help: &[
            "Deletes the entry from its file for good, the same delete the",
            "uninstall button does after its second press.",
        ],
        args: &[Arg::of("name", Shape::Rest)],
        does: Does::RemoveServer,
    },
    Command {
        name: "set_theme",
        about: "wear one of the preset themes",
        help: &[
            "The same write picking a theme on the panel does: the preset",
            "lands in the settings file and the colour lines that were",
            "overriding it go, so the whole palette follows.",
        ],
        args: &[Arg::of("theme", Shape::Value)],
        does: Does::Set {
            key: crate::settings::THEME,
            file: File::Window,
        },
    },
    Command {
        name: "set_font_conversation",
        about: "the conversation's font size",
        help: &[
            "Writes font_size in the window's settings file: the messages,",
            "and everything else in the transcript.",
        ],
        args: &[Arg::of("size", Shape::Value)],
        does: Does::Set {
            key: "font_size",
            file: File::Window,
        },
    },
    Command {
        name: "set_font_panes",
        about: "the panes' font size",
        help: &[
            "Writes pane_font_size in the window's settings file: the",
            "activity, plan, agents and file panes, and the settings panel.",
        ],
        args: &[Arg::of("size", Shape::Value)],
        does: Does::Set {
            key: "pane_font_size",
            file: File::Window,
        },
    },
    Command {
        name: "set_transparency_panels",
        about: "opacity of the panes, bars and menus",
        help: &[
            "Writes opacity in the window's settings file: everything with",
            "words on it. 1 is solid, the low end is glass.",
        ],
        args: &[Arg::of("value", Shape::Value)],
        does: Does::Set {
            key: "opacity",
            file: File::Window,
        },
    },
    Command {
        name: "set_transparency_window",
        about: "opacity of the space around the panes",
        help: &[
            "Writes window_opacity in the window's settings file: the empty",
            "space between the panes, where the desktop shows through. 0 is",
            "all the way to glass.",
        ],
        args: &[Arg::of("value", Shape::Value)],
        does: Does::Set {
            key: "window_opacity",
            file: File::Window,
        },
    },
    Command {
        name: "set_prompt_rows",
        about: "how many rows tall the box you type in is",
        help: &["Writes prompt_rows in the window's settings file."],
        args: &[Arg::of("rows", Shape::Value)],
        does: Does::Set {
            key: "prompt_rows",
            file: File::Window,
        },
    },
    Command {
        name: "set_color",
        about: "write one palette colour as #rrggbb",
        help: &[
            "The same write a pressed swatch takes: the value lands in the",
            "window's settings file under the colour's key and overrides",
            "its theme. The APPEARANCE section's footer names each swatch's",
            "key when it is pressed; accent, text, panel and bar are the",
            "big four.",
        ],
        args: &[
            Arg::of("key", Shape::ColourKey),
            Arg::of("value", Shape::Colour),
        ],
        does: Does::SetColour,
    },
    Command {
        name: "restore_appearance",
        about: "put every size, transparency and colour back to its default",
        help: &[
            "The restore under the palette: every appearance line in the",
            "settings file is commented out where it stands, so the next",
            "read falls back to what the window ships with. A colour tuned",
            "by hand loses its line too.",
        ],
        args: &[],
        does: Does::RestoreLooks,
    },
    Command {
        name: "settings",
        about: "open the settings panel, on a section by name",
        help: &[
            "Opens the panel, on the named section or where it was left.",
            "The sections are the rail's own, spelled in snake_case:",
            "agent, system_prompt, sessions, skills, mcp, commands,",
            "appearance.",
        ],
        args: &[Arg::maybe("section", Shape::Section)],
        does: Does::OpenPanel,
    },
    Command {
        name: "help",
        about: "list the commands, or explain one",
        help: &[
            "/help lists every command with what it does in one line.",
            "/help <name> prints the usage line and the whole story for",
            "that one.",
        ],
        args: &[Arg::maybe("command", Shape::Word)],
        does: Does::Help,
    },
];

/// The usage line without its prefix: `/set_theme <theme>`, with the note on
/// what each argument takes. The COMMANDS section's rows and `/help` both
/// read it, so the two cannot describe one command differently.
pub fn spelled(command: &Command) -> String {
    let mut line = format!("/{}", command.name);
    for arg in command.args {
        match arg.optional {
            true => line.push_str(&format!(" [{}]", arg.name)),
            false => line.push_str(&format!(" <{}>", arg.name)),
        }
    }
    if let Some(note) = dispatch::takes(command) {
        line.push_str(&format!(", {note}"));
    }
    line
}

/// The usage line a refused call answers with.
pub fn usage(command: &Command) -> String {
    format!("usage: {}", spelled(command))
}

/// What `/help <name>` prints, and what the COMMANDS section shows in the
/// column beside the list: the usage line, then the whole story.
pub fn manual(command: &Command) -> Vec<String> {
    let mut lines = vec![spelled(command), String::new()];
    lines.extend(command.help.iter().map(|line| (*line).to_string()));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{self, Deed, Doing};

    fn named(name: &str) -> &'static Command {
        ALL.iter()
            .find(|command| command.name == name)
            .unwrap_or_else(|| panic!("no /{name} in the registry"))
    }

    /// The registry is well formed: names are unique snake_case, every
    /// command has a line for the list and a manual behind it, and only the
    /// last argument swallows the rest of the line or may be left out.
    #[test]
    fn the_registry_is_well_formed() {
        for command in &ALL {
            assert!(
                command
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_'),
                "/{} is not snake_case",
                command.name
            );
            assert!(!command.about.is_empty(), "/{} says nothing", command.name);
            assert!(
                !command.help.is_empty(),
                "/{} has no manual",
                command.name
            );
            assert_eq!(
                ALL.iter()
                    .filter(|other| other.name == command.name)
                    .count(),
                1,
                "/{} twice",
                command.name
            );
            for (at, arg) in command.args.iter().enumerate() {
                let last = at + 1 == command.args.len();
                if matches!(arg.shape, Shape::Rest) || arg.optional {
                    assert!(last, "/{}: <{}> must come last", command.name, arg.name);
                }
            }
            let lines = manual(command);
            assert_eq!(lines[0], spelled(command));
        }
    }

    /// Every setting the panel can change has a command that writes the same
    /// key to the same file, walked off the panel's own tables so a setting
    /// added there without a command fails here.
    #[test]
    fn every_panel_setting_has_a_command() {
        let covers = |key: &str, file: File| {
            ALL.iter().any(|command| {
                matches!(command.does, Does::Set { key: k, file: f } if k == key && f == file)
            })
        };
        for (key, _) in settings::LOOKS {
            assert!(covers(key, File::Window), "no command writes {key}");
        }
        for (key, _) in settings::AGENT_SETTINGS {
            assert!(covers(key, File::Agent), "no command writes {key}");
        }
        // The endpoint is the panel's one typed field, not on a table.
        assert!(covers(crate::agent::ENDPOINT, File::Agent));
    }

    /// Every deed the panel can do, and every act its card buttons carry,
    /// has a command. The two matches are exhaustive on purpose: a variant
    /// added to either enum refuses to compile until it is named here, which
    /// is what makes a new capability without a command a broken build.
    #[test]
    fn every_panel_deed_has_a_command() {
        fn command_for(deed: &Deed) -> &'static str {
            match deed {
                Deed::TurnSkill { on: true, .. } => "skill_on",
                Deed::TurnSkill { on: false, .. } => "skill_off",
                Deed::RemoveSkill { .. } => "skill_remove",
                Deed::TurnServer { on: true, .. } => "mcp_on",
                Deed::TurnServer { on: false, .. } => "mcp_off",
                Deed::RemoveServer { .. } => "mcp_remove",
                Deed::AddServer { .. } => "mcp_add",
                // Writing the documents happens on the panel; the command
                // that covers both prompt-file deeds opens it there.
                Deed::SaveInstructions { .. } | Deed::RestorePrompt { .. } => {
                    "show_system_prompt"
                }
                Deed::ForgetSessions { .. } => "delete_session",
                Deed::RestoreLooks => "restore_appearance",
            }
        }
        fn command_for_button(doing: Doing) -> &'static str {
            match doing {
                Doing::Validate | Doing::Install => "skill_install",
                Doing::AddServer => "mcp_add",
                Doing::Restore => "restore_appearance",
            }
        }
        let deeds = [
            Deed::TurnSkill {
                dir: String::new(),
                on: true,
            },
            Deed::TurnSkill {
                dir: String::new(),
                on: false,
            },
            Deed::RemoveSkill {
                dir: String::new(),
                on: true,
            },
            Deed::TurnServer {
                name: String::new(),
                project: false,
                on: true,
            },
            Deed::TurnServer {
                name: String::new(),
                project: false,
                on: false,
            },
            Deed::RemoveServer {
                name: String::new(),
                project: false,
            },
            Deed::AddServer {
                name: String::new(),
                how: String::new(),
            },
            Deed::SaveInstructions {
                path: std::path::PathBuf::new(),
                text: String::new(),
            },
            Deed::RestorePrompt {
                path: std::path::PathBuf::new(),
                default: "",
            },
            Deed::ForgetSessions { ids: Vec::new() },
            Deed::RestoreLooks,
        ];
        for deed in &deeds {
            named(command_for(deed));
        }
        for doing in [Doing::Validate, Doing::Install, Doing::AddServer, Doing::Restore] {
            named(command_for_button(doing));
        }
    }
}
