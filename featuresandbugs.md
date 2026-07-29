when i open the app the mouse is like loading for a few seconds (10, 15 seconds) and icon does not appear, the app does open, but no icon on the side bar of ubuntu, then icon appear and mouse thinking stops loading, however the app opens instantly.

thinking, totally unclear, i want an animation somewhere, an orb thinking.


selection is not precise it has issues, like not selecting correctly the text.
still scroll is not correct, some text box exceed and i do not see the last part of messages.
for files, show better a side view.



llm status, separate into 2 different monitors: acutal session, and total overall.

total overall will have:
total tokens prefilled so far
total tokens generated so far
total cached prefill so far
average median decoding
average median prefill


and then on the actual session show
actual context used
total tool calls so far
single higest generated output so far


then another monitor for debugging:
total failed tool calls
(when i click one i want to see what were sent, and expected schema, this is debug)


remove the footer completely from the app.

when i double click i see a black thing, should be all green and also have some green lines up like something is wrong in the display.


improve tab vie, and show spaces, meaning when i drag i want to see a full part of where it will go, allow me full side or quadrants, logic is super hard for this for many reasons, first, if something is already splitted then should only allow me to split there.

make a config panel, where i setup:
endpoint, with key.
web-search nordvpn keys (hidden, that is user and pass)
enable/disable tor
enable/disable coding prompt and show how much it has in context (this will be a new md)
enable/disable custom AGENT.md (allow select file, this should paste it somewhere where this is injected, or paste it, and here visualize it, in the config)
skills panel:
a list of skills to enable/disable them, and each with how much context.
each skill with their official description and official repo.

improve tabs, the title is like... the tab should have a slight color like in the image provided.

overall:
Give it more life on icons, etc, find a library or whatever that allow you use icons, AND animations very lightweight.

the thinking orb i want is here:
https://github.com/Jakubantalik/thinking-orbs
this is react, meaning you will have to code your own in rust, use the one named <ThinkingOrb state="working" size={64} /> that one is when is active, when is not active make it just the shpere one spining, this one <ThinkingOrb state="searching" size={64} />

This means the double click when collapse has 66 px (because you will use 1 px of margin each) and put that icon on left top.

when i open the app, first thing is select folder i want to be.

Improve the workspace as i said, give it more tech view more cool, and visual and pleasurable.

for example the different metrics etc, in hardware for example, instead of showing bars show to 100% but in 4 dots cols, so 10.5 would be 10 bars full 4 dots each, with next one with only 2 dots (0,25,50,75,100 basically).

on hardware show only those bars, the rest info no.

In config panel also allow to setup transparency, and palette color in themes.

in each window, do not make it total square make it square, but on right limit, cut with a line at 45 degree small corner, so it gives more style.
the orb should be out of the bar, a square with the animation, clickable, if i click it, on the side left of course, show the list of all possible panels, those active already with a crox to close them, and those inactive you can drag them into the spaces.

allow a resize of spaces so, our grid allow 4 boxes, each with tabs, you should be able to resize those spaces however you want.
so this means allows [[x,x,x,x]] this is a flex, so can be on top 100% and under 2 x 50 width each, or 100% height, 2 x 500 height each on side.
SUper hard layout but i guess you understand what i mean do the correct logic and be sure is not buggy, i will test it deeply

In config allow a click "classic" view, which is like this:
chat result width 100%, 70% height, under left 50% plan, 30% height, side 50% agents, 30% height.
the input is always on bottom no resizable and there, single line, with scroll if you type too  much.

remove the minimize box tabs  (an arrow you added) because we will allow close panels directly
to remove a panel i simply drag it out of the window, or right click, close.
allow right click.

allow ctrl + a to select all on input, also allow to click anywhere to edit from there., in configuration allow edit how many vertical lines the input has.

make all this into a plan prioritize performance and speed, make it slow and calmly.


REMEMBER OF HIGH IMPORTANCE! isolation.

Allow an api expose for prompt, this means, not exacly an api but like CLI, so figure it out, meaning other CLIs should be able, IF CLIppy is open, to access to it by using it, and this should impact as prompt on the app, figure out if possible to do it as a service and protocol as well, the use case would be i do an external website that uses my local agent to do stuff (such as whatsapp, or torrent, etc).
For things you have doubts YOU TAKE IT EASY AND RESEARCH UP TO DATE 2026 JUL LIMIT! to see whats the best way to solve the issue do not jump into conclussions.


more added features:
in settings allow to manage sessions in memory
on the file list, add nice looking icons, for mds, js, py, etc etc
