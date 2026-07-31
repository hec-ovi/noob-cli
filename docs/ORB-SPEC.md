# The thinking orb, ported

The working frame is taken from the real source of
`github.com/Jakubantalik/thinking-orbs` (MIT), `src/engine/core.ts`,
`src/engine/orbits.ts`, `src/engine/profiles.ts` and `src/presets.ts`, read
rather than guessed. This file exists so the Rust port matches the animation
instead of approximating it. The resting frame is not upstream's any more (see
below) and the move between the two is NO0B's own.

## Why it needs no shader

Every frame is a list of z-sorted dots. NO0B already draws a clean
antialiased disc with `Panel::fill(rgba).radius(w / 2)` through the existing
rounded-rect SDF, and the same rect with no corner radius is a hard square. So
the whole animation is arithmetic that emits rects: no second pipeline, no WGSL,
no texture. The working state is drawn as discs and the idle state as squares,
and the corner radius is what says which: it travels with the move, so a dot
rounds off as it leaves the plate and squares up again as it settles.

At 64px the `working` state emits **516 discs** per frame (12 orbits, each 40
ghost dots plus 3 particles). That is well inside the rect buffer, which grows
by powers of two from 256.

## The two formations

One state is ported: `working` onto upstream's `orbits`. There is no third, and
the ASCII face loop that was going to fill the idle state is dropped (decision 3).

Preset at size 64, `orbits`: `speed 1.885`, `count 1`, `size 1`, so the base
profile is used unscaled.

The idle formation is NO0B's own and has been three things. It was the `orbits`
frame frozen at `t = 0`, which is twelve tilted ellipses standing still and reads
as scattered dots. It was then upstream's `globe`, a latitude and longitude
lattice on one sphere, thinned to 112 square dots. It is now a **square**: a
filled 11 by 11 plate of dots across the block, which is the mark this window
carries everywhere else, and flat and still in a way a ball is not.

Nothing about the plate reads the clock, so a resting window redraws the same
picture however long it has been up and still holds no wakeup deadline while it
rests.

The two radius keys the plate uses, 0.69 and 1.955, are what is left of the
`globe` preset at size 64 (`size 1.15` applied to `rBase` 0.6 and `rDepth` 1.7).
They are kept because they were tuned against this strip at this size, not
because the mode they came from is still here.

## Base profile (orbits)

| Name | Value | Meaning |
|---|---|---|
| `orbitN` | 12 | tilted circles |
| `ghostN` | 40 | dots tracing each circle's path |
| `ghostR` | 0.9 | ghost dot radius before scaling |
| `ghostA` | 0.5 | ghost dot alpha before depth |
| `particles` | 3 | dots running along each circle |
| `partR` | 1.2 | particle radius before depth |
| `partRDepth` | 1.6 | how much a particle grows as it comes forward |
| `rsPow` | 0.6 | radius scaling exponent |
| `rMin` | 0.3 | smallest radius drawn |

## Profile (the resting plate)

| Name | Value | Meaning |
|---|---|---|
| `side` | 11 | dots per side of the filled grid, so 121 in all |
| `rBase` | 0.69 | dot radius at the edge of the plate |
| `rDepth` | 1.955 | how much a dot grows towards the middle |
| `inkFar` | 0.62 | ink at the edge |
| `inkSpan` | 0.54 | how much darker it gets towards the middle |
| `rsPow` | 0.6 | radius scaling exponent |
| `rMin` | 0.3 | smallest radius drawn |

Eleven a side is what the strip has room for: the plate spans 24.6px at the
strip's 30, so the pitch is 2.46px against dots drawn 0.6px to 1.33px wide. Ten
leaves the middle sparse and twelve closes the gaps up.

`radiusScale(size, pow) = (size / 300).powf(pow)`, so at 64px the multiplier is
`(64/300)^0.6`. The radii were tuned for a 300pt frame and scale sub-linearly so
a small orb stays legible.

## The maths

Deterministic hash, so the arrangement is stable frame to frame:

```
hash(a, b) = fract(sin(a * 12.9898 + b * 78.233) * 43758.5453)
```

Centre `cx = cy = size / 2`, radius `R = (size / 2) * 0.82`.

Projection is a shared spin and tilt, orthographic, with `yaw = t * 0.12` and
`tilt = 0.3`:

```
x1 = x * cos(yaw) + z * sin(yaw)
z1 = -x * sin(yaw) + z * cos(yaw)
y1 = y * cos(tilt) - z1 * sin(tilt)
z2 = y * sin(tilt) + z1 * cos(tilt)
screen = (cx + x1, cy - y1),  depth = z2
```

Per orbit `orb` in `0..12`, with `h1 = hash(orb, 1.7)`, `h2 = hash(orb, 5.2)`,
`h3 = hash(orb, 8.9)`:

```
ro  = R * (0.45 + 0.52 * h1)          orbit radius
th  = h1 * 2pi
phi = acos(2 * h2 - 1)
n   = (sin(phi) * cos(th), cos(phi), sin(phi) * sin(th))     plane normal
u   = normalise((-n.y, n.x, 0))
v   = cross(n, u)
speed = (0.25 + 0.55 * h3) * (if h3 > 0.5 { 1 } else { -1 })
```

A point at angle `a` on that orbit is `(u * cos(a) + v * sin(a)) * ro`, projected
as above. `depth = (z / ro + 1) / 2`, clamped into 0..1 in practice.

Ghost path, `k` in `0..40`, angle `a = (k / 40) * 2pi`:

```
radius = 0.9 * rs
alpha  = 0.5 * (0.4 + 0.6 * depth)
ink    = 0.72
```

Particles, `m` in `0..3`, angle `a = t * speed + (m / 3) * 2pi + h2 * 6`:

```
radius = (1.2 + 1.6 * depth) * rs
alpha  = 1
ink    = 0.3 - 0.22 * depth
```

The plate does not go through the projection at all: a square seen at a tilt is a
rhombus, and the point of the formation is that it is square. `R` is its half
side, so its edges land where the circles' widest reach does. For `i, j` in
`0..side`:

```
u = -1 + 2 * i / (side - 1)
v = -1 + 2 * j / (side - 1)
screen = (cx + u * R, cy + v * R)
depth  = 1 - max(abs(u), abs(v))
z      = (2 * depth - 1) * R
radius = (0.69 + 1.955 * depth) * rs
alpha  = 1
ink    = 0.62 - 0.54 * depth
```

A flat plate has no depth of its own, so one is made from how far out a dot is,
by the larger of the two distances rather than the straight line between them:
that falls away in squares, so the shape of the brightness is the shape of the
formation. `z` is that spread back over the plate's own width, in pixels like the
orbits' own, so a dot travelling between the two can be sorted against one that
stayed where it was. At 30px the plate is 121 dots.

Then sort every dot by `z` ascending (far to near) and draw in that order,
skipping any with alpha below 0.02 and clamping radius to at least `rMin`. The
corner radius is `radius * morph`, so the plate's dots are hard squares and the
orbits' are discs.

## The move between them

A turn starting does not swap one frame for the other. `morph` is how far along
the move a frame is, 0 at the plate and 1 at the circles, and the two ends are
the two formations exactly: `morph == 0` builds the plate alone and `morph == 1`
builds the circles alone, so the resting frame stays clock-free and the working
frame is the arithmetic it always was.

In between, both are built and paired. Each of the 121 plate dots takes the
working dot at `index * 516 / 121` in the circles' own emission order, which is
deterministic, so a dot keeps the same partner every frame with nothing
remembered between them, and the stride spreads the plate across all twelve
circles rather than pouring it into the first three. Position, depth, radius,
alpha and ink are all linear between the pair. The 395 working dots left over
ride their own place with `alpha * morph`, so they come up out of nothing as the
plate opens out. The whole frame's fade goes `0.6` to `1` over the same move.

The move runs 300ms, nine frames at the orb's own 33ms deadline, in both
directions. Leaving is the direction that costs something: `orb_deadline` would
otherwise return `None` the instant the turn ended and leave the orb frozen
halfway back to its square, so it takes an `animating` flag that is the running
turn OR the orb still travelling. That is finite by construction, since the
transition steps to exactly zero and stops. `Morph` in `main.rs` measures from
the moment the turn started or ended rather than stepping by however long the
last wake took: an idle window blocks indefinitely, so a wake can be an hour
after the one before it, and stepping by that would arrive at the far end on the
first frame.

## Ink, and what it means here

The reference is greyscale on paper: `ink` is 0 for darkest, and on a dark
substrate it is mirrored to `1 - ink` so near dots read bright. NO0B is a dark
window, so mirror it, and map the resulting value through the theme's accent
rather than to grey. Near particles land bright, far ghost paths recede. Depth
is carried by radius and weight only, never by blur.

## Where it lives and when it runs

`gui/clippy/src/orb.rs`, drawn by `view::title_bar` into the square at the left
end of the title strip. That square is `view::ORB_W`, which is the strip's height,
30px, not the 66px block first sketched: the strip's text starts after it, so the
strip reads `[orb] NO0B \u{25b8} version` left to right. `radiusScale(size, pow)`
is passed that real size rather than a 64px result scaled down.

At 30px the working state is 516 discs a frame and the resting state 121 squares,
which the rectangle buffer holds by growing once to 1024. A frame partway through
the move is never more than the working count, because the plate's dots travel
into the circles rather than being drawn beside them.

**It must not free-run.** `noob-gpu` records that a previous version rendered
static text at 3,500 fps and spent a third of the graphics pipe doing it. So
`about_to_wait` holds a `WaitUntil` deadline (`orb_deadline`, 30 frames a second)
that exists only while `State::phase.busy()` or the orb is still travelling back
from a turn, and it is composed with the monitor's sampling deadline by `soonest`
rather than replacing it. Never `ControlFlow::Poll`.

The clock is `App::epoch`, passed into the scene as `Frame::clock` in seconds
rather than read inside it, so the same clock builds the same frame twice. `t` in
the formulas above is that multiplied by the preset speed of 1.885. The move's
own progress goes the same way, as `Frame::orb_morph`, which is `None` whenever
the orb is settled and the phase says at which end.

## Cost to watch

Text is re-shaped from scratch every frame. The orb itself is only rects, so it
is cheap, but anything that puts animated *text* on screen at the same rate wants
buffer caching first.
