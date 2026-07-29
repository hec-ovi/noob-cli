# The thinking orb, ported

Taken from the real source of `github.com/Jakubantalik/thinking-orbs` (MIT),
`src/engine/{core,orbits,profiles}.ts` and `src/presets.ts`, read rather than
guessed. This file exists so the Rust port matches the animation instead of
approximating it.

## Why it needs no shader

Every frame is a list of z-sorted discs. CLIppy already draws a clean
antialiased disc with `Panel::fill(rgba).radius(w / 2)` through the existing
rounded-rect SDF, and painter's order is just the order rects are pushed. So the
whole animation is arithmetic that emits rects: no second pipeline, no WGSL, no
texture.

At 64px the `working` state emits **516 discs** per frame (12 orbits, each 40
ghost dots plus 3 particles). That is well inside the rect buffer, which grows
by powers of two from 256.

## The state we need

`STATE_TO_MODE` maps `working` to the `orbits` mode. The idle state is not the
library's `searching` globe: it is the ASCII face loop in `docs/asciis`, per the
later decision, rendered as tinted dots. So only `orbits` is ported.

Preset at size 64: `speed 1.885`, `count 1`, `size 1`, so the base profile is
used unscaled.

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

Then sort every dot by `z` ascending (far to near) and draw in that order,
skipping any with alpha below 0.02 and clamping radius to at least `rMin`.

## Ink, and what it means here

The reference is greyscale on paper: `ink` is 0 for darkest, and on a dark
substrate it is mirrored to `1 - ink` so near dots read bright. CLIppy is a dark
window, so mirror it, and map the resulting value through the theme's accent
rather than to grey. Near particles land bright, far ghost paths recede. Depth
is carried by radius and weight only, never by blur.

## Where it lives and when it runs

A 66px block at the window's top left, above the panes: 64 plus a one pixel
margin each side. Idle plays the ASCII face loop; the orb replaces it while the
agent is working, with a transition between the two.

**It must not free-run.** `noob-gpu` records that a previous version rendered
static text at 3,500 fps and spent a third of the graphics pipe doing it. Follow
the avatar's precedent: extend `about_to_wait` with a `WaitUntil` deadline that
exists only while the orb is visible and animating, and stops at the end of a
turn. Never `ControlFlow::Poll`.

The clock is `App::epoch`, already sampled as elapsed milliseconds for the
avatar. `t` in the formulas above is seconds multiplied by the preset speed of
1.885.

## Cost to watch

Text is re-shaped from scratch every frame. The orb itself is only rects, so it
is cheap, but anything that puts animated *text* on screen at the same rate (the
ASCII idle loop is roughly 4,700 tinted cells) wants buffer caching first.
