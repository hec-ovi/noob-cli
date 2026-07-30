# The thinking orb, ported

Taken from the real source of `github.com/Jakubantalik/thinking-orbs` (MIT),
`src/engine/{core,orbits,lattice,profiles}.ts` and `src/presets.ts`, read rather
than guessed. This file exists so the Rust port matches the animation instead of
approximating it.

## Why it needs no shader

Every frame is a list of z-sorted discs. NO0B already draws a clean
antialiased disc with `Panel::fill(rgba).radius(w / 2)` through the existing
rounded-rect SDF, and painter's order is just the order rects are pushed. So the
whole animation is arithmetic that emits rects: no second pipeline, no WGSL, no
texture.

At 64px the `working` state emits **516 discs** per frame (12 orbits, each 40
ghost dots plus 3 particles). That is well inside the rect buffer, which grows
by powers of two from 256.

## The states we need

`STATE_TO_MODE` maps six states onto six modes. Two of them are ported: `working`
onto `orbits`, and the idle state onto `globe`, which upstream is what `searching`
uses. There is no third, and the ASCII face loop that was going to fill the idle
state is dropped (decision 3).

The idle state was the `orbits` frame frozen at `t = 0`, and it was wrong: twelve
tilted ellipses standing still read as scattered dots, not as an object. `globe`
is a latitude and longitude lattice on one sphere, so it closes a silhouette and
holds a ball in the corner while nothing is running. Upstream sweeps a scan
meridian across it, which is the only moving part of that mode and is left out
here: idle does not animate, and without the scan there is no clock term in the
frame at all, so the window still stops redrawing when a turn ends and still
holds no wakeup deadline while it rests.

Preset at size 64, `orbits`: `speed 1.885`, `count 1`, `size 1`, so the base
profile is used unscaled.

Preset at size 64, `globe`: `count 0.42`, `size 1.15`. `scaleCounts` takes the
square root of the count multiplier for a lattice pair, so the TOTAL dot count
scales by 0.42 and each side by 0.648: `latRings` 17 becomes 11 and `lonDensity`
44 becomes 29. `scaleRadii` multiplies every radius key by 1.15, so `rBase` 0.6
becomes 0.69 and `rDepth` 1.7 becomes 1.955. The `speed`, `scanMul` and `dimBase`
of that preset belong to the scan and are not ported. The 20 point preset was
tried at the strip's real 30px and is sparser (54 dots to 204); the denser one
reads as an object sooner, which is the whole point of the mode.

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

## Base profile (globe)

| Name | Value | Meaning |
|---|---|---|
| `latRings` | 17 | rings of latitude, pole to pole |
| `lonDensity` | 44 | dots around the widest ring |
| `rBase` | 0.6 | dot radius at the back of the sphere |
| `rDepth` | 1.7 | how much a dot grows as it comes forward |
| `inkFar` | 0.62 | ink at the back |
| `inkSpan` | 0.54 | how much darker it gets coming forward |
| `rsPow` | 0.6 | radius scaling exponent |
| `rMin` | 0.3 | smallest radius drawn |

`rBoost`, and the `scanMul` and `dimBase` of the preset, are the scan meridian
and are not ported.

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

The globe uses the same projection with `yaw = 0` (it does not turn) and
`tilt = 0.4`, which is the middle of the tilt upstream wobbles by 0.06 over time.
The points go in on the unit sphere and the projection scales them by `R`, so the
depth that comes back is already `-1..1`:

```
lat = -pi/2 + (li / latRings) * pi                for li in 0..=latRings
lonCount = max(1, round(abs(cos(lat)) * lonDensity))
lon = (lj / lonCount) * 2pi                        for lj in 0..lonCount
point  = (cos(lat) * cos(lon), sin(lat), cos(lat) * sin(lon))
depth  = (z + 1) / 2
radius = (0.69 + 1.955 * depth) * rs
alpha  = 1
ink    = 0.62 - 0.54 * depth
```

A ring's dot count follows the cosine of its latitude, so spacing along a ring
matches spacing between rings and the poles come out as one dot each. At 30px
that is 204 dots.

Then sort every dot by `z` ascending (far to near) and draw in that order,
skipping any with alpha below 0.02 and clamping radius to at least `rMin`.

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

At 30px the working state is 516 discs a frame and the resting state 204, which
the rectangle buffer holds by growing once to 1024.

**It must not free-run.** `noob-gpu` records that a previous version rendered
static text at 3,500 fps and spent a third of the graphics pipe doing it. So
`about_to_wait` holds a `WaitUntil` deadline (`orb_deadline`, 30 frames a second)
that exists only while `State::phase.busy()`, and it is composed with the
monitor's sampling deadline by `soonest` rather than replacing it. Never
`ControlFlow::Poll`.

The clock is `App::epoch`, passed into the scene as `Frame::clock` in seconds
rather than read inside it, so the same clock builds the same frame twice. `t` in
the formulas above is that multiplied by the preset speed of 1.885.

## Cost to watch

Text is re-shaped from scratch every frame. The orb itself is only rects, so it
is cheap, but anything that puts animated *text* on screen at the same rate wants
buffer caching first.
