# Showcase video, 0.11.16

One 6:33 cut from two screencasts, narrated over the Lemmino Cipher loop.
Everything in the beat table was read off a frame at that timestamp, not from
memory.

Sources, both 1188x813 and silent:

- `~/Videos/Screencasts/Screencast From 2026-08-05 16-02-09.mp4` (5:14)
- `~/Videos/Screencasts/Screencast From 2026-08-05 16-07-34.mp4` (2:38)

Renders land in `~/Videos/noob-showcase/`, outside the repo.

## The cut

| Master | Source | Why |
|---|---|---|
| 0:00 - 0:48 | v1 0:00 - 0:48 | |
| 0:48 - 0:50 | v1 0:50 - 0:52 | freeze at 0:48 dropped |
| 0:50 - 2:29 | v1 0:54 - 2:33 | freeze at 0:52 dropped |
| 2:29 - 5:07 | v1 2:36 - 5:14 | pause at 2:33 dropped |
| 5:07 - 6:17 | v2 0:14 - 1:24 | v2 head dropped |
| 6:17 - 6:33 | v2 1:58 - 2:14 | the wait dropped, tail after the result dropped |

```bash
ffmpeg -i "$V1" -i "$V2" -filter_complex "
[0:v]fps=30,crop=1188:812:0:0,setsar=1,split=4[a0][b0][c0][d0];
[1:v]fps=30,crop=1188:812:0:0,setsar=1,split=2[e0][f0];
[a0]trim=0:48,setpts=PTS-STARTPTS[a];   [b0]trim=50:52,setpts=PTS-STARTPTS[b];
[c0]trim=54:153,setpts=PTS-STARTPTS[c]; [d0]trim=156:314,setpts=PTS-STARTPTS[d];
[e0]trim=14:84,setpts=PTS-STARTPTS[e];  [f0]trim=118:134,setpts=PTS-STARTPTS[f];
[a][b][c][d][e][f]concat=n=6:v=1:a=0[v]" -map "[v]" \
  -c:v libx264 -preset slow -crf 16 -pix_fmt yuv420p -movflags +faststart \
  noob-showcase-master.mp4
```

The crop takes one row off the bottom because 813 is odd and yuv420p needs even
sides.

## Beats, verified

| Master | On screen |
|---|---|
| 0:05 | picker at `/home/hec/noob-workspace`, `new folder: p_` being typed |
| 0:12 | inside `noob-workspace/project_2`, hardware panel reads `sampling…` |
| 0:25 | right click menu open, Widgets submenu, ACTIVITY unchecked |
| 0:32 | ACTIVITY added as a second tab beside OUTPUT |
| 0:40 | typing `can you create a plan of 3 st` in the prompt |
| 1:01 | prompt sent, GPU starts climbing |
| 1:24 | activity rows: plan, write `tools_list.md`, plan, web init. GPU 65% |
| 1:36 | SESSION tab mid-drag, plan panel step 1 `[x]`, step 2 `[>]`, phase INFERRING |
| 1:56 | plan call popup: the todos JSON out, `plan: 0/3 done` back |
| 2:05 | websearch call popup: `init`, 23.4s, SearXNG answering, capability list |
| 2:26 | three steps `[x]`, file table: `tools_list.md`, `websearch_result.md`, `summary.md` |
| 2:50 | sub-agent asked for, AGENTS panel: `[1] Agent running all Perform 2 web searches` |
| 3:08 | third tab `[1] AGENT - OUTPUT`, main agent still answering |
| 3:28 | right click menu over the transcript, heading for settings |
| 3:41 | SETTINGS, SYSTEM PROMPT, and THE ENVIRONMENT BLOCK below it |
| 3:48 | SESSIONS, 4 rows with size, context share, first message |
| 3:53 | SKILLS, `1 SKILL INSTALLED: web search` |
| 4:03 | COMMANDS, `/set_reasoning`, `/set_context_window`, `/set_max_subagents`, `/set_endpoint` |
| 4:28 | APPEARANCE, main transparency 0.95, widget transparency 0.45 |
| 4:41 | noob-red worn, whole window and the hardware bars red, GPU 96% |
| 4:48 | themes card, noob-cool back on |
| 5:05 | CONTEXT: phase FINISHED, 12 requests, 10 tool calls, last prefill 5,813 |
| 5:23 | FILES added as a fourth tab, three files listed, `summary.md` open |
| 5:39 | the sub-agent's own transcript: two parallel searches, seven fetches, two writes |
| 6:03 | layout rearranged, CONTEXT docked next to PLAN, HARDWARE top right |
| 6:23 | sub-agent gone from AGENTS, its tab closed, main agent phase INFERRING |
| 6:31 | the result: 3 files with sizes and descriptions, 13 requests, 6,426 context |

## Narration

Eleven v3. Stability Natural, speed a touch under 1.0. Tags are v3 audio tags,
not SSML, so `[pause]` rather than `<break>`. One file per block, named
`vo-01.mp3` and so on, each one dropped at its in-point.

**01 — in at 0:04**

> [softly] This is NO0B. [pause] You point it at a folder, [pause] and it opens
> with the agent already inside. [pause] No terminal. [pause] No browser.
> [drawn out] One window, drawn on the GPU, sitting on your desktop.

**02 — in at 0:26**

> [calm] Every panel here is one you asked for. [pause] Right click anywhere and
> the list is waiting: output, activity, plan, agents, hardware, context,
> sessions, files. [pause] Tick one, and it takes a tab.

**03 — in at 0:43**

> [softly] So ask for something real. [pause] List your tools. [pause] Run a web
> search. [pause] Write the summary. [drawn out] Three steps, in one sentence.

**04 — in at 1:03**

> [calm] Watch the right hand side. [pause] The GPU climbs while the model
> thinks, [pause] and the plan writes itself into its own panel. [pause] One
> line per step, [pause] ticking off as the work lands.

**05 — in at 1:26**

> [softly] Activity keeps the receipts. [pause] Every tool call in order, with
> the file it touched and the second it happened. [pause] Pull the dividers
> around. [pause] Drag a tab into another dock. [drawn out] The layout is yours,
> and it arrives on the very next frame.

**06 — in at 1:57**

> [calm] Click any row [pause] and the whole call opens over the window. [pause]
> What went out, [pause] what came back, [pause] how long it took. [pause] Select
> it, copy it, keep it.

**07 — in at 2:27**

> [softly] Three steps, three files, [pause] handed back as a table of what it
> made. [drawn out] And the plan is all ticks.

**08 — in at 2:50**

> [calm] Now the good part. [pause] Ask for a sub-agent, [pause] and it spins up
> beside the main one, [pause] with its own tab and its own transcript. [pause]
> It goes off researching in the background [drawn out] while you carry on
> talking to the agent in front of you.

**09 — in at 3:30**

> [softly] Right click, settings. [pause] The system prompt it is actually
> running, [pause] and the environment block it computes fresh for every request.
> [pause] Sessions you can mark and clear out. [pause] Skills. [pause] MCP
> servers.

**10 — in at 4:05**

> [calm] Commands is the whole surface in one list. [pause] The endpoint,
> reasoning, the context window, [pause] how many sub-agents may run at once.
> [pause] Each one with its own page of help beside it.

**11 — in at 4:29**

> [softly] Appearance is live. [pause] Text size. [pause] Transparency, for the
> window and for the panels on their own. [pause] Then the themes. [pause]
> Matrix green. [pause] Cool cyan. [pause] Red. [drawn out] The whole window
> turns while you are looking at it.

**12 — in at 5:04**

> [calm] Context knows exactly what the run cost. [pause] Twelve requests,
> [pause] ten tool calls, [pause] five thousand eight hundred and thirteen tokens
> in the last prefill.

**13 — in at 5:23**

> [softly] The files panel lists what the agent wrote. [pause] Click one and it
> opens right there, [pause] markdown or code. [drawn out] Nothing to leave, and
> nothing else to open.

**14 — in at 5:42**

> [calm] Meanwhile the sub-agent has been busy. [pause] Two searches in parallel,
> [pause] a handful of fetches, [pause] three files of its own.

**15 — in at 6:18**

> [softly] Then it finishes, [pause] closes itself, [pause] and hands the whole
> thing back to the agent you were talking to.

**16 — in at 6:26**

> [drawn out] NO0B. [pause] Rust, one window, your own model. [pause] On GitHub,
> as noob-cli.

## Sound effects

Generated on Eleven's sound effects, one file each, `sfx-01.wav` and so on.
Each sits under the music, quiet, a little seasoning on the click.

| File | In at | Prompt | Length |
|---|---|---|---|
| sfx-01 | 0:11 | soft synthetic interface confirm, single low tone with a short shimmer tail, clean and quiet | 1.5s |
| sfx-02 | 0:25 | tiny UI panel open, soft airy whoosh with a light click at the front, very short | 1.0s |
| sfx-03 | 0:44 | distant mechanical keyboard typing, soft and muffled, a few keys, no clatter | 3.0s |
| sfx-04 | 1:03 | deep electronic hum swelling in, machine waking up, warm and low, no melody | 3.0s |
| sfx-05 | 1:37 | short soft drag and dock sound, muted thud with an airy tail, interface feel | 1.0s |
| sfx-06 | 1:56 | soft modal popup open, gentle upward whoosh with a glassy edge | 1.2s |
| sfx-07 | 2:27 | quiet completion chime, two soft notes rising, clean digital, no reverb wash | 2.0s |
| sfx-08 | 2:51 | a second presence powering up, low synth swell with a soft sparkle on top | 2.5s |
| sfx-09 | 3:29 | settings panel sliding open, smooth low whoosh, soft and slow | 1.5s |
| sfx-10 | 4:40 | colour shift shimmer, soft granular sweep, brief and dreamy, no impact | 2.0s |
| sfx-11 | 5:24 | light paper or file card sliding into place, soft and dry, very short | 1.0s |
| sfx-12 | 6:22 | soft power down, gentle descending hum fading out | 2.0s |
| sfx-13 | 6:30 | final resolve, warm low pad with a single soft bell, calm and clean | 3.0s |

## Music

`~/Music/experimental/youtubemusic/LEMMiNO - Cipher (BGM) (EXTENDED 1 HOUR [SEAMLESS]).mp3`,
trimmed to the cut. Four seconds of fade in, six of fade out, held around
-24 dB under the voice, and side-chained so it steps back whenever the narration
speaks and comes back up in the gaps.

## Assembling

`assemble.sh` next to the renders takes the master cut, the `vo-*.mp3` blocks and
the `sfx-*.wav` files, places each at its in-point, and writes the final mix.
Run it again whenever a block is regenerated.
