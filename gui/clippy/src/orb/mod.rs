//! The thinking orb: the one animated thing in the window.
//!
//! The turning frame is a port of one mode of `thinking-orbs` (MIT), following
//! `docs/ORB-SPEC.md`, which was written off that source rather than guessed.
//! Every frame is a list of dots projected orthographically and sorted far to
//! near. Depth is carried by how big a dot is and how much weight its colour
//! has, never by blur.
//!
//! It needs no shader and no second pipeline. A dot is one rectangle through the
//! rounded-rect distance field the window already draws every panel with: with
//! the corner radius at half the width it is a disc, and with no corner radius at
//! all it is a square. Painter's order is the order rectangles are pushed.
//!
//! Two formations, and no third. While a turn is running it is `orbits`: twelve
//! tilted circles of path dots sharing one spin, with three runners chasing each
//! circle, drawn as discs. At rest it is `square`: a filled eleven by eleven
//! plate of dots across the block, drawn as squares, once and never again. The
//! resting formation was a dotted globe before, and before that the orbits frame
//! frozen at zero; a square is the mark the corner of this window carries
//! everywhere else, and it is flat and still in a way a ball is not.
//!
//! The two are not swapped for one another. Between them is one move: each
//! resting dot is paired with a dot of the turning frame and travels to it,
//! rounding off from a square to a disc on the way, while the turning dots
//! nobody was paired with come up out of nothing. `morph` is how far along that
//! move a frame is, 0 at the plate and 1 at the orbits, and the two ends are the
//! two formations exactly.
//!
//! The maths is here and the drawing is in [`crate::view`], so a frame can be
//! asserted without a GPU: [`discs`] is a pure function of the block it is given,
//! the clock, and how far the move has got.

use noob_draw::{Panel, Rect};

use crate::skin::Skin;

use std::f32::consts::TAU;

/// Tilted circles. Twelve is the preset, and it is what makes a handful of
/// rings read as a sphere rather than as a handful of rings.
const ORBITS: usize = 12;

/// Dots tracing each circle's own path. They do not move along it: they are the
/// path, and the runners are what moves.
const GHOSTS: usize = 40;

/// Dots running around each circle.
const PARTICLES: usize = 3;

/// A path dot's radius and alpha before depth and before scaling.
const GHOST_R: f32 = 0.9;
const GHOST_A: f32 = 0.5;

/// A path dot's ink. Flat, because a path is a path at any depth; what changes
/// with depth is how much of it there is.
const GHOST_INK: f32 = 0.72;

/// A runner's radius, and how much of it it gains coming forward.
const PART_R: f32 = 1.2;
const PART_R_DEPTH: f32 = 1.6;

/// A runner's ink at the back of the sphere, and how much darker (so, here,
/// brighter) it gets coming forward.
const PART_INK: f32 = 0.3;
const PART_INK_DEPTH: f32 = 0.22;

/// The resting plate's side, in dots. It is filled, so the count is the square
/// of this.
///
/// Eleven is what the strip has room for. The plate spans 24.6 pixels at the
/// title strip's 30, so eleven a side is a 2.46 pixel pitch against dots that
/// are drawn between 0.6 and 1.33 pixels wide: the gaps stay open and it reads
/// as dots rather than as a filled box. Ten leaves the middle looking sparse at
/// a 2.73 pitch, and twelve closes the gaps up at 2.24.
const SIDE: usize = 11;

/// A resting dot's radius at the edge of the plate and how much it gains coming
/// in towards the middle, before scaling. Carried over from the globe the plate
/// replaced, which took them from the reference's 0.6 and 1.7 at its 64 point
/// preset's size multiplier of 1.15.
const REST_R: f32 = 0.69;
const REST_R_DEPTH: f32 = 1.955;

/// A resting dot's ink at the edge of the plate, and how much darker (so, here,
/// brighter) it gets coming in. Unlike a path dot, whose ink is flat, every dot
/// here is on the one surface, so ink is the whole of what gives the plate a
/// middle to look at.
const REST_INK_FAR: f32 = 0.62;
const REST_INK_SPAN: f32 = 0.54;

/// The frame the radii were tuned on, and the exponent they scale by.
///
/// Sub-linear on purpose: a 30 pixel orb scaled linearly off a 300 point drawing
/// would be dots too small to have a colour at all. At the title strip's height
/// the multiplier is about a quarter rather than a tenth.
const RS_FRAME: f32 = 300.0;
const RS_POW: f32 = 0.6;

/// The smallest radius drawn, in pixels, and the faintest alpha. Below either,
/// the reference stops drawing the dot rather than drawing something that is not
/// there.
const R_MIN: f32 = 0.3;
const ALPHA_FLOOR: f32 = 0.02;

/// How much of the block the arrangement spans, as a fraction of half its size.
/// It is the sphere the circles are laid out in, and it is the plate's half
/// side, so the plate's edges land where the circles' widest reach does and the
/// two formations are the same size on screen.
const FILL: f32 = 0.82;

/// The shared spin, per unit of clock, and the one tilt the whole arrangement is
/// seen at.
const YAW_RATE: f32 = 0.12;
const TILT: f32 = 0.3;

/// The preset's speed: seconds are multiplied by this to get the clock the
/// formulas use.
const SPEED: f32 = 1.885;

/// How much of itself the resting plate keeps.
///
/// Not from the reference, which has no resting state: it is the step down that
/// says nothing is running without leaving the corner blank. The move between
/// the two formations brings it back up to the full.
const RESTING: f32 = 0.6;

/// The smallest block worth drawing in.
///
/// Under this the sphere is a few pixels across and every dot is on the floor
/// radius, so it reads as a smudge; and the containment the tests pin (no disc
/// leaves its block) stops holding, because the floor radius stops being small
/// next to the block. Only reached when the compositor hands back a window
/// shorter than the title strip asked for.
const MIN_BLOCK: f32 = 8.0;

/// One dot, before it becomes a rectangle.
///
/// `ink` is the reference's greyscale value, where 0 is the darkest mark on
/// paper. This window is dark, so it is mirrored when the dot becomes a colour.
#[derive(Clone, Copy, Debug)]
struct Dot {
    x: f32,
    y: f32,
    /// Depth after the tilt, and what the list is sorted by. In pixels in both
    /// formations, so a frame partway between them sorts a travelling dot
    /// against a dot that stayed where it was and gets an answer that means
    /// something.
    z: f32,
    radius: f32,
    alpha: f32,
    ink: f32,
}

/// One circle's plane: two unit vectors in it and how big it is.
struct Orbit {
    u: [f32; 3],
    v: [f32; 3],
    ro: f32,
}

/// The deterministic hash the arrangement is built from, in [0, 1).
///
/// Deterministic is the whole point: the twelve circles are laid out from it
/// every frame, so the same arrangement has to come back every frame. Computed
/// at double precision because the last step multiplies by 43758 and keeps the
/// fraction, which in single precision is most of the answer thrown away.
fn hash(a: f32, b: f32) -> f32 {
    let h = (a as f64 * 12.9898 + b as f64 * 78.233).sin() * 43758.5453;
    (h - h.floor()) as f32
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    // A circle whose normal points straight up has no left, so pick one rather
    // than dividing by zero and putting every dot of it at NaN.
    if len < 1e-6 {
        return [1.0, 0.0, 0.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Where a point lands on screen, and how deep it is.
///
/// Orthographic, and the spin and the tilt are shared by every dot of a frame,
/// which is what makes twelve circles read as one solid rather than as twelve
/// circles. Screen y grows downward, so the projected height is subtracted.
///
/// `scale` is what the point is measured in: the orbits hand it points already
/// in pixels and scale them by one. The depth that comes back is in the same
/// units the point went in as. The resting plate does not come through here at
/// all, because a square seen at a tilt is a rhombus.
fn project(p: [f32; 3], yaw: f32, tilt: f32, centre: (f32, f32), scale: f32) -> (f32, f32, f32) {
    let (sy, cy) = (yaw.sin(), yaw.cos());
    let (st, ct) = (tilt.sin(), tilt.cos());
    let x1 = p[0] * cy + p[2] * sy;
    let z1 = -p[0] * sy + p[2] * cy;
    let y1 = p[1] * ct - z1 * st;
    let z2 = p[1] * st + z1 * ct;
    (centre.0 + x1 * scale, centre.1 - y1 * scale, z2)
}

/// A point at angle `a` on one circle: where it is, and its depth as 0 at the
/// back of that circle and 1 at the front.
fn place(a: f32, orbit: &Orbit, yaw: f32, centre: (f32, f32)) -> (f32, f32, f32, f32) {
    let (c, s) = (a.cos(), a.sin());
    let p = [
        (orbit.u[0] * c + orbit.v[0] * s) * orbit.ro,
        (orbit.u[1] * c + orbit.v[1] * s) * orbit.ro,
        (orbit.u[2] * c + orbit.v[2] * s) * orbit.ro,
    ];
    let (x, y, z) = project(p, yaw, TILT, centre, 1.0);
    let depth = ((z / orbit.ro + 1.0) * 0.5).clamp(0.0, 1.0);
    (x, y, z, depth)
}

/// One value on the way to another.
fn mix(from: f32, to: f32, morph: f32) -> f32 {
    from + (to - from) * morph
}

/// What a block is worth: how much a dot is scaled by, how wide the arrangement
/// is from its middle, and where that middle is. `None` for a block too small to
/// draw anything readable in.
///
/// The three numbers both formations are laid out from, taken in one place so
/// they cannot be taken two ways.
fn scale(block: Panel) -> Option<(f32, f32, (f32, f32))> {
    let size = block.w.min(block.h);
    if size < MIN_BLOCK {
        return None;
    }
    let rs = (size / RS_FRAME).powf(RS_POW);
    let sphere = size * 0.5 * FILL;
    let centre = (block.x + block.w * 0.5, block.y + block.h * 0.5);
    Some((rs, sphere, centre))
}

/// Every dot of one frame, sorted far to near.
///
/// `seconds` is time since the window opened; the formulas run on it multiplied
/// by the preset speed. At rest the clock is not read at all: the plate has no
/// term in it, so a resting window redraws the same picture however long it has
/// been up, which is what lets the window stop redrawing entirely.
///
/// The two ends are their own formations and nothing else, which is what keeps
/// the resting frame clock-free and the turning frame the arithmetic it always
/// was. Only a frame strictly between them pays for both.
fn dots(block: Panel, seconds: f32, morph: f32) -> Vec<Dot> {
    let Some((rs, sphere, centre)) = scale(block) else {
        return Vec::new();
    };
    let mut out = if morph <= 0.0 {
        square(rs, sphere, centre)
    } else if morph >= 1.0 {
        orbits(seconds * SPEED, rs, sphere, centre)
    } else {
        between(seconds * SPEED, rs, sphere, centre, morph)
    };
    // Far to near, so a near dot covers the path behind it. Rectangles inside a
    // layer are painted in the order they are pushed, so this sort IS the depth
    // buffer.
    out.sort_by(|a, b| a.z.total_cmp(&b.z));
    out
}

/// The working frame: twelve tilted circles of path dots, with runners on them.
fn orbits(t: f32, rs: f32, sphere: f32, centre: (f32, f32)) -> Vec<Dot> {
    let yaw = t * YAW_RATE;
    let mut out = Vec::with_capacity(ORBITS * (GHOSTS + PARTICLES));
    for index in 0..ORBITS {
        let index = index as f32;
        let (h1, h2, h3) = (hash(index, 1.7), hash(index, 5.2), hash(index, 8.9));
        let th = h1 * TAU;
        let phi = (2.0 * h2 - 1.0).clamp(-1.0, 1.0).acos();
        // The circle's plane, as its normal and two unit vectors lying in it.
        let n = [phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin()];
        let u = unit([-n[1], n[0], 0.0]);
        let orbit = Orbit {
            u,
            v: cross(n, u),
            ro: sphere * (0.45 + 0.52 * h1),
        };
        // Half of them run backwards, so the sphere does not read as one wheel.
        let speed = (0.25 + 0.55 * h3) * if h3 > 0.5 { 1.0 } else { -1.0 };

        for k in 0..GHOSTS {
            let a = (k as f32 / GHOSTS as f32) * TAU;
            let (x, y, z, depth) = place(a, &orbit, yaw, centre);
            out.push(Dot {
                x,
                y,
                z,
                radius: GHOST_R * rs,
                alpha: GHOST_A * (0.4 + 0.6 * depth),
                ink: GHOST_INK,
            });
        }

        // The runners are what motion is, and this frame only exists while
        // something is running, so every circle gets its three.
        for m in 0..PARTICLES {
            let a = t * speed + (m as f32 / PARTICLES as f32) * TAU + h2 * 6.0;
            let (x, y, z, depth) = place(a, &orbit, yaw, centre);
            out.push(Dot {
                x,
                y,
                z,
                radius: (PART_R + PART_R_DEPTH * depth) * rs,
                alpha: 1.0,
                ink: PART_INK - PART_INK_DEPTH * depth,
            });
        }
    }
    out
}

/// The resting frame: a filled square of dots, with no clock in it anywhere.
///
/// `sphere` is the square's half side, so the plate's edges land where the
/// circles' widest reach does and it fits the block on the same terms. There is
/// no projection: a square seen at a tilt is a rhombus, and the point of this
/// formation is that it is square.
///
/// A flat plate has no depth of its own, so one is made out of how far from the
/// middle a dot is, by the larger of the two distances rather than by the
/// straight line between them. That is itself a square, so a dot's size and
/// weight fall away in rings that are the shape of the formation. Depth is then
/// spread back over the plate's own width as the sort key, so a travelling dot
/// can be sorted against a turning one.
fn square(rs: f32, sphere: f32, centre: (f32, f32)) -> Vec<Dot> {
    let mut out = Vec::with_capacity(SIDE * SIDE);
    let last = (SIDE - 1) as f32;
    for row in 0..SIDE {
        for column in 0..SIDE {
            let u = -1.0 + 2.0 * column as f32 / last;
            let v = -1.0 + 2.0 * row as f32 / last;
            let depth = 1.0 - u.abs().max(v.abs());
            out.push(Dot {
                x: centre.0 + u * sphere,
                y: centre.1 + v * sphere,
                z: (depth * 2.0 - 1.0) * sphere,
                radius: (REST_R + REST_R_DEPTH * depth) * rs,
                alpha: 1.0,
                ink: REST_INK_FAR - REST_INK_SPAN * depth,
            });
        }
    }
    out
}

/// The frame partway through the move between the two formations.
///
/// Not one picture fading into another: every dot of the plate is paired with
/// one dot of the turning frame and travels to it, so the plate opens out into
/// the circles. The pairing is a stride through the turning frame's own emission
/// order, which is deterministic and the same every frame, so a dot keeps the
/// partner it had last frame without anything being remembered between them. The
/// stride is what spreads the plate across all twelve circles: paired in order
/// instead, the whole plate would pour into the first three.
///
/// The turning dots left over, which is most of them, stay where they belong and
/// come up out of nothing as the move runs.
fn between(t: f32, rs: f32, sphere: f32, centre: (f32, f32), morph: f32) -> Vec<Dot> {
    let work = orbits(t, rs, sphere, centre);
    let rest = square(rs, sphere, centre);
    let mut out: Vec<Dot> = work
        .iter()
        .map(|dot| Dot {
            alpha: dot.alpha * morph,
            ..*dot
        })
        .collect();
    for (index, from) in rest.iter().enumerate() {
        let landing = index * work.len() / rest.len();
        let to = work[landing];
        out[landing] = Dot {
            x: mix(from.x, to.x, morph),
            y: mix(from.y, to.y, morph),
            z: mix(from.z, to.z, morph),
            radius: mix(from.radius, to.radius, morph),
            alpha: mix(from.alpha, to.alpha, morph),
            ink: mix(from.ink, to.ink, morph),
        };
    }
    out
}

/// The frame to draw, in painter's order.
///
/// `block` is the square the title strip keeps for it, `seconds` is the clock,
/// and `morph` is how far the orb is between its two formations: 0 is the
/// resting plate, 1 is the turning orbits, and anything between is the move from
/// one to the other. Every dot lands inside `block`, so the caller does not have
/// to clip.
///
/// Shape is decided here rather than in the maths, and it travels with the move:
/// a corner radius of half the width is a disc and none at all is a square, both
/// the same rectangle through the same distance field, so a dot rounds off as it
/// leaves the plate and squares up again as it comes back. The turning orb is
/// round because it is moving and a moving square strobes at this size; the
/// resting one is square because it is one still frame, where a hard mark reads
/// as a mark instead of as a smudge.
///
/// The ink is mirrored and tinted here rather than in the maths: the reference is
/// greyscale on paper, where 0 is the darkest mark, and this is a dark window, so
/// a dot's weight is `1 - ink` and it is that much of the theme's accent. Alpha
/// stays the reference's, scaled down as a whole while the window rests and
/// brought back up over the move.
pub fn discs(block: Panel, seconds: f32, morph: f32, skin: &Skin) -> Vec<Rect> {
    let morph = morph.clamp(0.0, 1.0);
    let fade = mix(RESTING, 1.0, morph);
    let [r, g, b, _] = skin.orb;
    dots(block, seconds, morph)
        .into_iter()
        .filter_map(|dot| {
            let alpha = dot.alpha * fade;
            if alpha < ALPHA_FLOOR {
                return None;
            }
            let radius = dot.radius.max(R_MIN);
            let weight = (1.0 - dot.ink).clamp(0.0, 1.0);
            let fill = [r * weight, g * weight, b * weight, alpha];
            let mark = Panel::new(dot.x - radius, dot.y - radius, radius * 2.0, radius * 2.0)
                .fill(fill);
            Some(mark.radius(radius * morph))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::view::{ORB_W, TITLE_H};

    fn block() -> Panel {
        Panel::new(0.0, 0.0, ORB_W, TITLE_H)
    }

    fn skin() -> Skin {
        Skin::from(&Config::default())
    }

    /// The palette the pinned frame below was measured in. Its own rather than
    /// the window's default, which moves when the shipped theme does: the claim
    /// is about the geometry and the fades, not about which theme ships.
    fn pinned_skin() -> Skin {
        Skin::from(&crate::config::theme("noob-matrix").expect("a preset"))
    }

    /// A scene as numbers, so two of them can be compared. [`Rect`] is a shader
    /// struct and has no equality of its own.
    fn frozen(rects: &[Rect]) -> Vec<([f32; 4], [f32; 4], [f32; 4])> {
        rects
            .iter()
            .map(|rect| (rect.xywh(), rect.rgba(), rect.extra()))
            .collect()
    }

    /// The turning frame is the working formation, and the resting one is the
    /// plate. Named rather than written as bare numbers, because `1.0` and `0.0`
    /// at the third argument of [`discs`] say nothing on their own.
    const TURNING: f32 = 1.0;
    const RESTING_FRAME: f32 = 0.0;

    /// Every dot is laid out from a hash of its orbit's index, so the same
    /// moment has to draw the same picture. If it did not, the orbits would
    /// rearrange themselves between two redraws of the same frame.
    #[test]
    fn the_same_moment_draws_the_same_orb() {
        let (skin, block) = (skin(), block());
        for seconds in [0.0, 0.4, 7.25, 613.5] {
            let once = discs(block, seconds, TURNING, &skin);
            let twice = discs(block, seconds, TURNING, &skin);
            assert_eq!(frozen(&once), frozen(&twice), "at {seconds}s");
        }
    }

    /// And a different moment draws a different one, or the animation is a still
    /// image with a clock attached.
    #[test]
    fn a_turn_actually_turns() {
        let (skin, block) = (skin(), block());
        let first = discs(block, 0.0, TURNING, &skin);
        let later = discs(block, 0.5, TURNING, &skin);
        assert_ne!(frozen(&first), frozen(&later));
    }

    /// Painter's order is the whole depth test, so the list has to arrive sorted
    /// back to front. Out of order, a dot at the back of the sphere is drawn over
    /// the runner in front of it.
    #[test]
    fn the_dots_are_ordered_far_to_near() {
        for seconds in [0.0, 1.5, 96.0] {
            let dots = dots(block(), seconds, TURNING);
            assert!(dots.len() > 1);
            for pair in dots.windows(2) {
                assert!(pair[0].z <= pair[1].z, "{:?} then {:?}", pair[0], pair[1]);
            }
        }
    }

    /// Neither floor may be crossed by anything that reaches the screen: a dot
    /// under a third of a pixel is not a dot, and one under two percent alpha is
    /// not a colour. And each formation is drawn in its own shape: the turning
    /// orb is discs, the resting one is squares.
    #[test]
    fn nothing_drawn_is_fainter_or_smaller_than_the_floor() {
        let skin = skin();
        for (morph, working) in [(TURNING, true), (RESTING_FRAME, false)] {
            for seconds in [0.0, 2.75, 41.0] {
                for disc in discs(block(), seconds, morph, &skin) {
                    let [_, _, w, h] = disc.xywh();
                    assert!(w / 2.0 >= R_MIN - 1e-6, "radius {}", w / 2.0);
                    assert!((w - h).abs() < 1e-6, "a dot's box is square: {w}x{h}");
                    assert!(disc.rgba()[3] >= ALPHA_FLOOR, "alpha {}", disc.rgba()[3]);
                    // Turning: a disc, not a box, or the animation is 500 tiny
                    // squares. Resting: a box, not a disc, and the corner radius
                    // is the whole of what says so.
                    let corner = disc.extra()[0];
                    match working {
                        true => assert!((corner - w / 2.0).abs() < 1e-6, "{:?}", disc.extra()),
                        false => assert_eq!(corner, 0.0, "{:?}", disc.extra()),
                    }
                }
            }
        }
    }

    /// The block is the strip's height and the strip's text starts after it, so
    /// a disc outside the block is a disc on the window's name. Both formations
    /// are sized to fit rather than clipped, and this is what says so.
    ///
    /// Partway through the move as well, which is not free: a dot in there is at
    /// a place neither formation put it. It holds because both its place and its
    /// radius are the same mix of two dots that each fit, and a mix of two
    /// numbers inside the block is inside the block.
    #[test]
    fn every_disc_lands_inside_its_block() {
        let skin = skin();
        for panel in [
            block(),
            Panel::new(0.0, 0.0, ORB_W, 12.0),
            Panel::new(200.0, 40.0, 30.0, 30.0),
            Panel::new(0.0, 0.0, 64.0, 64.0),
            Panel::new(0.0, 0.0, MIN_BLOCK, MIN_BLOCK),
        ] {
            for morph in [RESTING_FRAME, 0.15, 0.5, 0.9, TURNING] {
                let discs = discs(panel, 3.5, morph, &skin);
                assert!(!discs.is_empty(), "{panel:?} draws nothing");
                for disc in discs {
                    let [x, y, w, h] = disc.xywh();
                    assert!(x >= panel.x - 1e-4, "{disc:?} left of {panel:?}");
                    assert!(y >= panel.y - 1e-4, "{disc:?} above {panel:?}");
                    assert!(x + w <= panel.x + panel.w + 1e-4, "{disc:?} right of {panel:?}");
                    assert!(y + h <= panel.y + panel.h + 1e-4, "{disc:?} below {panel:?}");
                }
            }
        }
    }

    /// A block too small for a sphere draws nothing rather than a smudge. Only
    /// reached when the compositor hands back a window shorter than the strip.
    #[test]
    fn a_block_too_small_to_read_draws_nothing() {
        let skin = skin();
        for size in [0.0, 1.0, MIN_BLOCK - 0.5] {
            assert!(discs(Panel::new(0.0, 0.0, ORB_W, size), 1.0, TURNING, &skin).is_empty());
            assert!(discs(Panel::new(0.0, 0.0, size, TITLE_H), 1.0, TURNING, &skin).is_empty());
        }
    }

    /// Both formations' own numbers, pinned. Working is twelve circles of forty
    /// path dots with three runners each, so 516 discs a frame; resting is the
    /// plate, which is eleven by eleven and has nothing to do with the circles.
    /// A frame partway between them is somewhere in between and never more than
    /// the working count: the plate's dots travel into the circles rather than
    /// being drawn alongside them, and the circles' own dots are only there once
    /// they are bright enough to be worth a rectangle.
    ///
    /// It also says what the rectangle buffer has to hold. It grows by powers of
    /// two from 256, so 516 discs plus a window's worth of panels takes it to
    /// 1024 once and never again, and the move never asks for more than that.
    #[test]
    fn each_formation_draws_the_whole_of_its_own_profile() {
        let skin = skin();
        let working = discs(block(), 1.0, TURNING, &skin);
        let resting = discs(block(), 1.0, RESTING_FRAME, &skin);
        assert_eq!(working.len(), ORBITS * (GHOSTS + PARTICLES));
        assert_eq!(working.len(), 516);
        assert_eq!(resting.len(), SIDE * SIDE);
        assert_eq!(resting.len(), 121);
        for morph in [0.02, 0.1, 0.5, 0.8, 0.99] {
            let moving = discs(block(), 1.0, morph, &skin).len();
            assert!((SIDE * SIDE..=516).contains(&moving), "{moving} discs at {morph}");
        }
    }

    /// The resting formation has been three different objects; the turning orb
    /// is the frame it always was, to the last decimal.
    ///
    /// Pinned against numbers taken off the build before the first of those
    /// changes rather than off this one: the count, the sum of every field of
    /// every disc, and three discs read out in full. The two formations share a
    /// scale and a block, and the move puts a multiply in front of every fade
    /// and every corner, so anything that moves the turning frame lands here.
    #[test]
    fn the_turning_orb_is_the_frame_it_was_before_the_resting_one_changed() {
        let (skin, block) = (pinned_skin(), block());
        // seconds, the sum, and the first, middle and last disc's box.
        for (seconds, sum, first, middle, last) in [
            (
                0.0f32,
                16483.265885,
                [11.871071, 14.018538, 0.6, 0.6],
                [19.127548, 8.214276, 0.6, 0.6],
                [17.528929, 15.381463, 0.6, 0.6],
            ),
            (
                1.0,
                16483.265906,
                [10.02061, 12.531108, 0.6, 0.6],
                [10.681802, 25.64781, 1.0023558, 1.0023558],
                [19.379395, 16.868889, 0.6, 0.6],
            ),
            (
                7.25,
                16483.265895,
                [15.969178, 14.995388, 0.6, 0.6],
                [5.6036844, 21.626549, 0.6, 0.6],
                [13.430822, 14.404611, 0.6, 0.6],
            ),
        ] {
            let frame = discs(block, seconds, TURNING, &skin);
            assert_eq!(frame.len(), 516, "at {seconds}s");
            let total: f64 = frame
                .iter()
                .flat_map(|rect| {
                    let (xywh, rgba, extra) = (rect.xywh(), rect.rgba(), rect.extra());
                    xywh.into_iter()
                        .chain(rgba)
                        .chain(extra)
                        .collect::<Vec<f32>>()
                })
                .map(f64::from)
                .sum();
            assert!((total - sum).abs() < 1e-4, "at {seconds}s the sum is {total}");
            for (index, want) in [(0usize, first), (257, middle), (515, last)] {
                let got = frame[index].xywh();
                for (a, b) in got.iter().zip(&want) {
                    assert!((a - b).abs() < 1e-5, "at {seconds}s disc {index} is {got:?}");
                }
            }
        }
    }

    /// At rest it is one frame with no clock in it, so two moments an hour apart
    /// are the same picture. This is the whole reason the window can stop
    /// redrawing when a turn ends.
    #[test]
    fn resting_is_the_same_frame_at_every_moment() {
        let (skin, block) = (skin(), block());
        let first = discs(block, 0.0, RESTING_FRAME, &skin);
        for seconds in [0.016, 9.5, 3600.0] {
            assert_eq!(
                frozen(&first),
                frozen(&discs(block, seconds, RESTING_FRAME, &skin)),
                "at {seconds}s"
            );
        }
    }

    /// How far out the furthest dot of a frame sits in each of twelve wedges
    /// around the middle of the block, as a fraction of the arrangement's own
    /// half width. A silhouette read off the drawn discs, so it is the outline
    /// somebody looking at the strip sees rather than the numbers behind it.
    fn rim(panel: Panel, morph: f32, skin: &Skin) -> [f32; 12] {
        let size = panel.w.min(panel.h);
        let sphere = size * 0.5 * FILL;
        let centre = (panel.x + panel.w * 0.5, panel.y + panel.h * 0.5);
        let mut out = [0.0f32; 12];
        for disc in discs(panel, 1.0, morph, skin) {
            let [x, y, w, h] = disc.xywh();
            let (dx, dy) = (x + w * 0.5 - centre.0, y + h * 0.5 - centre.1);
            let wedge = ((dy.atan2(dx) + TAU) / TAU * 12.0) as usize % 12;
            out[wedge] = out[wedge].max(dx.hypot(dy) / sphere);
        }
        out
    }

    /// The resting orb is a square, and the assertion is on the silhouette
    /// rather than on the dots: a plate of round dots is still a square and a
    /// grid of square dots arranged in a ball is not, so the shape of one dot
    /// says nothing about the shape of the formation.
    ///
    /// Cut the block into twelve wedges around its middle. Every one of them
    /// holds a dot out at the edge, so the outline is closed and there is no gap
    /// in it. Four of them, the ones the diagonals run through, reach half again
    /// as far as the rest, because that is what a corner is; the other eight
    /// stop at the flat of a side. A circle of any radius reaches the same
    /// distance in all twelve, which is what this rejects.
    ///
    /// The turning orb closes no outline at all: the widest of its circles stops
    /// short of the edge and the wedges between them are empty.
    #[test]
    fn the_resting_orb_is_a_square_and_the_turning_one_is_no_shape_at_all() {
        let skin = skin();
        for panel in [block(), Panel::new(12.0, 40.0, 300.0, 300.0)] {
            let resting = rim(panel, RESTING_FRAME, &skin);
            assert!(
                resting.iter().all(|reach| *reach > 0.97),
                "the resting orb has a gap in its edge: {resting:?} in {panel:?}"
            );
            // The diagonals are wedges 1, 4, 7 and 10: a wedge spans thirty
            // degrees from zero, so forty-five lands inside the second of them.
            for (wedge, reach) in resting.iter().enumerate() {
                match wedge % 3 {
                    1 => assert!(*reach > 1.3, "wedge {wedge} has no corner: {resting:?}"),
                    _ => assert!(*reach < 1.15, "wedge {wedge} is not a flat side: {resting:?}"),
                }
            }

            let working = rim(panel, TURNING, &skin);
            assert!(
                working.iter().all(|reach| *reach < 0.97),
                "the turning orb reached the edge: {working:?} in {panel:?}"
            );
        }
    }

    /// And it is a filled plate rather than an outline: eleven evenly spaced
    /// columns of eleven dots, one at every crossing, spanning the block's own
    /// middle. The silhouette above cannot tell those apart, since an outline
    /// has the same rim.
    #[test]
    fn the_resting_orb_is_a_filled_grid_of_dots() {
        let skin = skin();
        let panel = Panel::new(0.0, 0.0, 300.0, 300.0);
        let sphere = 300.0 * 0.5 * FILL;
        let pitch = 2.0 * sphere / (SIDE - 1) as f32;
        let mut seen = vec![vec![false; SIDE]; SIDE];
        for disc in discs(panel, 1.0, RESTING_FRAME, &skin) {
            let [x, y, w, h] = disc.xywh();
            let (dx, dy) = (x + w * 0.5 - 150.0 + sphere, y + h * 0.5 - 150.0 + sphere);
            let (column, row) = ((dx / pitch).round(), (dy / pitch).round());
            assert!((dx - column * pitch).abs() < 1e-3, "{dx} is off the grid");
            assert!((dy - row * pitch).abs() < 1e-3, "{dy} is off the grid");
            let cell = &mut seen[row as usize][column as usize];
            assert!(!*cell, "two dots at row {row} column {column}");
            *cell = true;
        }
        assert!(seen.iter().flatten().all(|filled| *filled), "the plate has a hole: {seen:?}");
    }

    /// The runners are the brightest thing in the turning orb and the paths are
    /// faint behind them, which is the mirrored ink working: a near runner's ink
    /// is the darkest mark on paper and the brightest dot on a dark window.
    ///
    /// The resting plate carries the same reading in its own way: a dot in the
    /// middle of it is brighter and bigger than one out at a corner, which is
    /// the only thing giving a flat grid of dots somewhere to look.
    #[test]
    fn depth_is_carried_by_weight_in_both_formations() {
        let skin = skin();
        let weight = |rect: &Rect| {
            let [r, g, b, a] = rect.rgba();
            (r + g + b) * a
        };
        let working = discs(block(), 1.0, TURNING, &skin);
        // A runner is at full alpha and a path dot never is, so the two are
        // separable without knowing where any of them landed.
        let heaviest = |rects: &[Rect], runner: bool| {
            rects
                .iter()
                .filter(|rect| (rect.rgba()[3] >= 1.0 - 1e-6) == runner)
                .map(weight)
                .fold(0.0f32, f32::max)
        };
        assert!(
            heaviest(&working, true) > heaviest(&working, false) * 2.0,
            "the runners do not stand out from the paths"
        );

        let resting = discs(block(), 1.0, RESTING_FRAME, &skin);
        // The list arrives far to near, so the last disc is the middle of the
        // plate and the first is a corner of it.
        assert!(
            weight(resting.last().expect("a plate")) > weight(&resting[0]) * 2.0,
            "the middle of the plate is not brighter than its corners"
        );
        // And every dot is the accent's hue turned down, never a colour of its
        // own: no channel may be over the accent's.
        let [r, g, b, _] = skin.orb;
        for disc in working.iter().chain(&resting) {
            let [dr, dg, db, _] = disc.rgba();
            assert!(dr <= r + 1e-6 && dg <= g + 1e-6 && db <= b + 1e-6, "{:?}", disc.rgba());
        }
    }

    /// The same frame in a bigger block is the same picture scaled, not a
    /// different arrangement, and the dots grow sub-linearly with it.
    #[test]
    fn a_bigger_block_holds_the_same_arrangement_with_bigger_dots() {
        let skin = skin();
        let small = discs(Panel::new(0.0, 0.0, 30.0, 30.0), 2.0, TURNING, &skin);
        let large = discs(Panel::new(0.0, 0.0, 120.0, 120.0), 2.0, TURNING, &skin);
        assert_eq!(small.len(), large.len());
        let widest = |rects: &[Rect]| rects.iter().map(|r| r.xywh()[2]).fold(0.0f32, f32::max);
        assert!(widest(&large) > widest(&small), "the dots grow");
        // Sub-linear: four times the block is nowhere near four times the dot.
        assert!(widest(&large) < widest(&small) * 4.0, "and not by the full factor");
    }

    /// The move is a move: a dot of the plate leaves the place the plate put it,
    /// travels in a straight line to the place its partner in the circles holds,
    /// and arrives there. It does not fade out while something else fades in
    /// over it, which is what "the dots accommodate" rules out.
    ///
    /// Asserted on the dots rather than on the drawn rectangles, because the
    /// frame is sorted by depth before it is drawn and a travelling dot changes
    /// its place in that order as it goes.
    #[test]
    fn a_resting_dot_travels_to_the_orbit_it_is_paired_with() {
        let (rs, sphere, centre) = scale(block()).expect("the strip's own block");
        let t = 1.0 * SPEED;
        let (rest, work) = (
            square(rs, sphere, centre),
            orbits(t, rs, sphere, centre),
        );
        for index in [0usize, 37, 60, 99, 120] {
            let landing = index * work.len() / rest.len();
            let (from, to) = (rest[index], work[landing]);
            let span = (to.x - from.x).hypot(to.y - from.y);
            assert!(span > 1.0, "dot {index} is paired with where it already is");

            let mut travelled = 0.0f32;
            for morph in [0.0f32, 0.1, 0.35, 0.6, 0.85, 1.0] {
                let dot = between(t, rs, sphere, centre, morph)[landing];
                let gone = (dot.x - from.x).hypot(dot.y - from.y);
                let left = (to.x - dot.x).hypot(to.y - dot.y);
                // On the line between the two, since the two legs add up to the
                // whole of it, and that far along it.
                assert!((gone + left - span).abs() < 1e-3, "dot {index} left the line");
                assert!((gone - span * morph).abs() < 1e-3, "dot {index} at {morph}");
                assert!(gone >= travelled, "dot {index} went back at {morph}");
                travelled = gone;
            }
            assert!((travelled - span).abs() < 1e-3, "dot {index} stopped short");
        }
    }

    /// At the two ends of the move, `between` is not asked at all: the plate and
    /// the circles are their own formations, which is what keeps the resting
    /// frame free of the clock and the turning frame the arithmetic that is
    /// pinned above. Anything past either end is that end.
    #[test]
    fn the_ends_of_the_move_are_the_two_formations_themselves() {
        let (skin, block) = (skin(), block());
        for (morph, end) in [(-2.0, RESTING_FRAME), (1.4, TURNING)] {
            assert_eq!(
                frozen(&discs(block, 3.0, morph, &skin)),
                frozen(&discs(block, 3.0, end, &skin)),
                "at {morph}"
            );
        }
    }

    /// The corners come off as the dots take up the orbit, and go back on as
    /// they settle: one rectangle through one distance field, so the shape is
    /// the corner radius and nothing else. Half the width is a disc.
    #[test]
    fn the_dots_round_off_over_the_move() {
        let skin = skin();
        for morph in [0.0f32, 0.2, 0.5, 0.75, 1.0] {
            for disc in discs(block(), 2.0, morph, &skin) {
                let corner = disc.extra()[0];
                let round = corner / (disc.xywh()[2] * 0.5);
                assert!((round - morph).abs() < 1e-5, "{round} round at {morph}");
            }
        }
    }

    /// The circles' own dots are not there at the start of the move and are all
    /// the way there at the end of it, so the plate opens out into them instead
    /// of the frame filling up in one step. Read off the whole frame's weight,
    /// which climbs the whole way.
    #[test]
    fn the_turning_dots_come_up_out_of_nothing() {
        let skin = skin();
        let ink = |morph: f32| {
            discs(block(), 2.0, morph, &skin)
                .iter()
                .map(|disc| {
                    let [r, g, b, a] = disc.rgba();
                    (r + g + b) * a
                })
                .sum::<f32>()
        };
        let mut last = ink(RESTING_FRAME);
        for morph in [0.1f32, 0.3, 0.5, 0.7, 0.9, 1.0] {
            let now = ink(morph);
            assert!(now > last, "the frame lost weight at {morph}: {now} after {last}");
            last = now;
        }
        // Twice the plate's own weight by the end. The circles are four times
        // the dots but most of them are faint paths, and the plate is drawn at
        // full alpha at its own dimmer fade.
        assert!(last > ink(RESTING_FRAME) * 1.9, "the circles never arrived");
    }
}
