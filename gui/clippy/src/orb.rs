//! The thinking orb: the one animated thing in the window.
//!
//! A port of the `orbits` mode of `thinking-orbs` (MIT), following
//! `docs/ORB-SPEC.md`, which was written off that source rather than guessed.
//! Twelve tilted circles of dots share one centre, one spin and one tilt; every
//! dot is projected orthographically, the whole list is sorted far to near, and
//! each dot is drawn as a disc. Depth is carried by how big a dot is and how much
//! weight its colour has, never by blur.
//!
//! It needs no shader and no second pipeline. A disc is a rectangle with its
//! corner radius set to half its width, through the rounded-rect distance field
//! the window already draws every panel with, and painter's order is the order
//! rectangles are pushed.
//!
//! Two states, and no third. While a turn is running the globe turns and three
//! runners chase each circle. At rest the same globe is drawn once, frozen and
//! fainter, with no runners: the corner is never empty and never moving for no
//! reason, and the runners arriving is what says the agent is thinking. The ASCII
//! face loop that used to fill the idle state is gone.
//!
//! The maths is here and the drawing is in [`crate::view`], so a frame can be
//! asserted without a GPU: [`discs`] is a pure function of the block it is given,
//! the clock, and whether there is a turn to animate.

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

/// How much of the block the sphere spans, as a fraction of half its size.
const FILL: f32 = 0.82;

/// The shared spin, per unit of clock, and the one tilt the whole arrangement is
/// seen at.
const YAW_RATE: f32 = 0.12;
const TILT: f32 = 0.3;

/// The preset's speed: seconds are multiplied by this to get the clock the
/// formulas use.
const SPEED: f32 = 1.885;

/// How much of itself the resting globe keeps.
///
/// Not from the reference, which had no resting state: it is the step down that
/// says nothing is running without leaving the corner blank.
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
    /// Depth after the tilt, in pixels, and what the list is sorted by.
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
/// Orthographic, and the spin and the tilt are shared by every circle, which is
/// what makes twelve of them read as one solid. Screen y grows downward, so the
/// projected height is subtracted.
fn project(p: [f32; 3], yaw: f32, centre: (f32, f32)) -> (f32, f32, f32) {
    let (sy, cy) = (yaw.sin(), yaw.cos());
    let (st, ct) = (TILT.sin(), TILT.cos());
    let x1 = p[0] * cy + p[2] * sy;
    let z1 = -p[0] * sy + p[2] * cy;
    let y1 = p[1] * ct - z1 * st;
    let z2 = p[1] * st + z1 * ct;
    (centre.0 + x1, centre.1 - y1, z2)
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
    let (x, y, z) = project(p, yaw, centre);
    let depth = ((z / orbit.ro + 1.0) * 0.5).clamp(0.0, 1.0);
    (x, y, z, depth)
}

/// Every dot of one frame, sorted far to near.
///
/// `seconds` is time since the window opened; the formulas run on it multiplied
/// by the preset speed. At rest the clock is ignored entirely and the frame is
/// the one at zero, so a resting window redraws the same picture however long it
/// has been up.
fn dots(block: Panel, seconds: f32, working: bool) -> Vec<Dot> {
    let size = block.w.min(block.h);
    if size < MIN_BLOCK {
        return Vec::new();
    }
    let t = if working { seconds * SPEED } else { 0.0 };
    let yaw = t * YAW_RATE;
    let rs = (size / RS_FRAME).powf(RS_POW);
    let sphere = size * 0.5 * FILL;
    let centre = (block.x + block.w * 0.5, block.y + block.h * 0.5);

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

        // The runners are what motion is. A frozen one would be three dots
        // sitting on a path claiming to run, so at rest there are none.
        if !working {
            continue;
        }
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
    // Far to near, so a near dot covers the path behind it. Rectangles inside a
    // layer are painted in the order they are pushed, so this sort IS the depth
    // buffer.
    out.sort_by(|a, b| a.z.total_cmp(&b.z));
    out
}

/// The frame to draw, as discs, in painter's order.
///
/// `block` is the square the title strip keeps for it, `seconds` is the clock,
/// and `working` is whether there is a turn to animate. Every disc lands inside
/// `block`, so the caller does not have to clip.
///
/// The ink is mirrored and tinted here rather than in the maths: the reference is
/// greyscale on paper, where 0 is the darkest mark, and this is a dark window, so
/// a dot's weight is `1 - ink` and it is that much of the theme's accent. Alpha
/// stays the reference's, scaled down as a whole while the window rests.
pub fn discs(block: Panel, seconds: f32, working: bool, skin: &Skin) -> Vec<Rect> {
    let fade = if working { 1.0 } else { RESTING };
    let [r, g, b, _] = skin.orb;
    dots(block, seconds, working)
        .into_iter()
        .filter_map(|dot| {
            let alpha = dot.alpha * fade;
            if alpha < ALPHA_FLOOR {
                return None;
            }
            let radius = dot.radius.max(R_MIN);
            let weight = (1.0 - dot.ink).clamp(0.0, 1.0);
            let fill = [r * weight, g * weight, b * weight, alpha];
            Some(
                Panel::new(dot.x - radius, dot.y - radius, radius * 2.0, radius * 2.0)
                    .fill(fill)
                    .radius(radius),
            )
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

    /// A scene as numbers, so two of them can be compared. [`Rect`] is a shader
    /// struct and has no equality of its own.
    fn frozen(rects: &[Rect]) -> Vec<([f32; 4], [f32; 4], [f32; 4])> {
        rects
            .iter()
            .map(|rect| (rect.xywh(), rect.rgba(), rect.extra()))
            .collect()
    }

    /// Every dot is laid out from a hash of its orbit's index, so the same
    /// moment has to draw the same picture. If it did not, the sphere would
    /// rearrange itself between two redraws of the same frame.
    #[test]
    fn the_same_moment_draws_the_same_orb() {
        let (skin, block) = (skin(), block());
        for seconds in [0.0, 0.4, 7.25, 613.5] {
            let once = discs(block, seconds, true, &skin);
            let twice = discs(block, seconds, true, &skin);
            assert_eq!(frozen(&once), frozen(&twice), "at {seconds}s");
        }
    }

    /// And a different moment draws a different one, or the animation is a still
    /// image with a clock attached.
    #[test]
    fn a_turn_actually_turns() {
        let (skin, block) = (skin(), block());
        let first = discs(block, 0.0, true, &skin);
        let later = discs(block, 0.5, true, &skin);
        assert_ne!(frozen(&first), frozen(&later));
    }

    /// Painter's order is the whole depth test, so the list has to arrive sorted
    /// back to front. Out of order, a dot at the back of the sphere is drawn over
    /// the runner in front of it.
    #[test]
    fn the_dots_are_ordered_far_to_near() {
        for seconds in [0.0, 1.5, 96.0] {
            let dots = dots(block(), seconds, true);
            assert!(dots.len() > 1);
            for pair in dots.windows(2) {
                assert!(pair[0].z <= pair[1].z, "{:?} then {:?}", pair[0], pair[1]);
            }
        }
    }

    /// Neither floor may be crossed by anything that reaches the screen: a disc
    /// under a third of a pixel is not a dot, and one under two percent alpha is
    /// not a colour.
    #[test]
    fn nothing_drawn_is_fainter_or_smaller_than_the_floor() {
        let skin = skin();
        for working in [true, false] {
            for seconds in [0.0, 2.75, 41.0] {
                for disc in discs(block(), seconds, working, &skin) {
                    let [_, _, w, h] = disc.xywh();
                    assert!(w / 2.0 >= R_MIN - 1e-6, "radius {}", w / 2.0);
                    assert!((w - h).abs() < 1e-6, "a disc is square: {w}x{h}");
                    assert!(disc.rgba()[3] >= ALPHA_FLOOR, "alpha {}", disc.rgba()[3]);
                    // A disc, not a box: the corner radius is half its width, or
                    // the orb is 500 tiny squares.
                    assert!((disc.extra()[0] - w / 2.0).abs() < 1e-6, "{:?}", disc.extra());
                }
            }
        }
    }

    /// The block is the strip's height and the strip's text starts after it, so
    /// a disc outside the block is a disc on the window's name. The sphere is
    /// sized to fit rather than clipped, and this is what says so.
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
            for working in [true, false] {
                let discs = discs(panel, 3.5, working, &skin);
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
            assert!(discs(Panel::new(0.0, 0.0, ORB_W, size), 1.0, true, &skin).is_empty());
            assert!(discs(Panel::new(0.0, 0.0, size, TITLE_H), 1.0, true, &skin).is_empty());
        }
    }

    /// The profile's own numbers, pinned. Twelve circles of forty path dots is
    /// the globe, and three runners each is the animation, so working is 516
    /// discs a frame and resting is the 480 that are the paths.
    ///
    /// It also says what the rectangle buffer has to hold. It grows by powers of
    /// two from 256, so 516 discs plus a window's worth of panels takes it to
    /// 1024 once and never again.
    #[test]
    fn working_draws_the_whole_profile_and_resting_draws_only_the_paths() {
        let skin = skin();
        let working = discs(block(), 1.0, true, &skin);
        let resting = discs(block(), 1.0, false, &skin);
        assert_eq!(working.len(), ORBITS * (GHOSTS + PARTICLES));
        assert_eq!(working.len(), 516);
        assert_eq!(resting.len(), ORBITS * GHOSTS);
        assert!(resting.len() < working.len());
    }

    /// At rest it is one frozen frame, so two moments an hour apart are the same
    /// picture. This is the whole reason the window can stop redrawing when a
    /// turn ends.
    #[test]
    fn resting_is_the_same_frame_at_every_moment() {
        let (skin, block) = (skin(), block());
        let first = discs(block, 0.0, false, &skin);
        for seconds in [0.016, 9.5, 3600.0] {
            assert_eq!(
                frozen(&first),
                frozen(&discs(block, seconds, false, &skin)),
                "at {seconds}s"
            );
        }
    }

    /// The runners are the brightest thing in the orb and the paths are faint
    /// behind them, which is the mirrored ink working: a near runner's ink is
    /// the darkest mark on paper and the brightest dot on a dark window.
    ///
    /// Resting has no runners and less alpha, so it is fainter on both counts,
    /// which is the whole visible difference between the two states.
    #[test]
    fn the_runners_are_the_brightest_thing_and_resting_gives_them_up() {
        let skin = skin();
        let weight = |rects: &[Rect]| {
            rects
                .iter()
                .map(|rect| {
                    let [r, g, b, a] = rect.rgba();
                    (r + g + b) * a
                })
                .fold(0.0f32, f32::max)
        };
        let working = discs(block(), 1.0, true, &skin);
        let resting = discs(block(), 1.0, false, &skin);
        assert!(
            weight(&working) > weight(&resting) * 2.0,
            "{} against {}",
            weight(&working),
            weight(&resting)
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
        let small = discs(Panel::new(0.0, 0.0, 30.0, 30.0), 2.0, true, &skin);
        let large = discs(Panel::new(0.0, 0.0, 120.0, 120.0), 2.0, true, &skin);
        assert_eq!(small.len(), large.len());
        let widest = |rects: &[Rect]| rects.iter().map(|r| r.xywh()[2]).fold(0.0f32, f32::max);
        assert!(widest(&large) > widest(&small), "the dots grow");
        // Sub-linear: four times the block is nowhere near four times the dot.
        assert!(widest(&large) < widest(&small) * 4.0, "and not by the full factor");
    }
}
