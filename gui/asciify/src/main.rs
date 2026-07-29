//! GIF to the text animation CLIppy plays.
//!
//! Run once, at authoring time, and commit what it prints. The window never
//! decodes a GIF: a decoder shipped to every user to read one file that never
//! changes is a decoder nobody needed, and the CLI's own budget discipline
//! applies here too even though the number is bigger.
//!
//! ```text
//! ./dev.sh avatar docs/asciis/clippy-black-1.gif gui/clippy/avatar/clippy.txt
//! ```
//!
//! ## Why the shape it produces looks the way it does
//!
//! A character cell is about twice as tall as it is wide, so a square image
//! sampled on a square grid comes out squashed to half height. The row count
//! is derived from the column count and that ratio rather than asked for, which
//! is the one thing every naive version of this gets wrong.
//!
//! Transparency is not darkness. A GIF's transparent pixels are "whatever is
//! behind this", and averaging them in as black draws a halo around the
//! subject. They are excluded from the average, and a cell that is entirely
//! transparent is a space.

use std::io::Write;

/// Dark to light. Enough steps to read as shading, few enough that each step
/// is visibly different at one character per cell.
const RAMP: &[u8] = b" .:-=+*#%@";

struct Args {
    input: String,
    output: Option<String>,
    cols: usize,
    /// Invert the ramp, for a subject that is dark on a light background.
    invert: bool,
    /// Cells dimmer than this fraction of full scale are blank. Without it,
    /// the compression artifacts around a subject read as a grey box.
    floor: f32,
}

fn main() -> std::process::ExitCode {
    const USAGE: &str = "usage: asciify <in.gif> [out.txt] [--cols N] [--invert] [--floor F]";
    let mut args = Args {
        input: String::new(),
        output: None,
        cols: 40,
        invert: false,
        floor: 0.12,
    };
    let mut positional = Vec::new();
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--cols" => match argv.next().and_then(|v| v.parse().ok()) {
                Some(cols) if cols > 0 => args.cols = cols,
                _ => return fail("--cols needs a positive number", USAGE),
            },
            "--floor" => match argv.next().and_then(|v| v.parse().ok()) {
                Some(floor) => args.floor = floor,
                None => return fail("--floor needs a number between 0 and 1", USAGE),
            },
            "--invert" => args.invert = true,
            other if other.starts_with("--") => {
                return fail(&format!("unknown flag {other:?}"), USAGE);
            }
            other => positional.push(other.to_string()),
        }
    }
    let mut positional = positional.into_iter();
    match positional.next() {
        Some(input) => args.input = input,
        None => return fail("no input GIF", USAGE),
    }
    args.output = positional.next();

    match convert(&args) {
        Ok(text) => {
            match &args.output {
                Some(path) => {
                    if let Err(e) = std::fs::write(path, &text) {
                        return fail(&format!("cannot write {path}: {e}"), USAGE);
                    }
                    let frames = text.lines().filter(|l| l.starts_with("f ")).count();
                    println!("{path}: {frames} frames, {} bytes", text.len());
                }
                None => {
                    let _ = std::io::stdout().write_all(text.as_bytes());
                }
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => fail(&e, USAGE),
    }
}

fn fail(message: &str, usage: &str) -> std::process::ExitCode {
    eprintln!("asciify: {message}\n{usage}");
    std::process::ExitCode::from(2)
}

/// One decoded frame's worth of luminance and coverage, in image coordinates.
struct Surface {
    width: usize,
    height: usize,
    /// Luminance 0..=255 per pixel.
    lum: Vec<u8>,
    /// Whether the pixel is part of the picture at all.
    solid: Vec<bool>,
}

impl Surface {
    fn snapshot(&self) -> Surface {
        Surface {
            width: self.width,
            height: self.height,
            lum: self.lum.clone(),
            solid: self.solid.clone(),
        }
    }
}

fn convert(args: &Args) -> Result<String, String> {
    let file = std::fs::File::open(&args.input)
        .map_err(|e| format!("cannot read {}: {e}", args.input))?;
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options
        .read_info(file)
        .map_err(|e| format!("{} is not a GIF this can read: {e}", args.input))?;

    let (width, height) = (decoder.width() as usize, decoder.height() as usize);
    if width == 0 || height == 0 {
        return Err(String::from("the GIF has no pixels"));
    }

    // The canvas persists between frames: a GIF frame may cover only part of
    // it and may ask for what was there before to stay. Composing every frame
    // onto one canvas is what makes a partial frame legible on its own.
    let mut canvas = Surface {
        width,
        height,
        lum: vec![0; width * height],
        solid: vec![false; width * height],
    };

    // Every frame is composed before any is sampled, because the crop has to
    // be the same for all of them: cropping each frame to its own subject
    // makes the subject jump around inside the panel instead of moving.
    let mut composed: Vec<(u32, Surface)> = Vec::new();
    let mut frames = 0;
    while let Some(frame) = decoder
        .read_next_frame()
        .map_err(|e| format!("frame {frames}: {e}"))?
    {
        compose(&mut canvas, frame);
        // GIF delays are hundredths of a second. Zero means "as fast as you
        // can", which every renderer since has read as ten.
        let delay_ms = match frame.delay {
            0 => 100,
            hundredths => u32::from(hundredths) * 10,
        };
        composed.push((delay_ms, canvas.snapshot()));
        frames += 1;
    }
    if frames == 0 {
        return Err(String::from("the GIF has no frames"));
    }

    let crop = bounds(composed.iter().map(|(_, surface)| surface));
    let cols = args.cols.min(crop.width.max(1));
    // Half, because a character cell is about twice as tall as it is wide.
    let rows = ((cols as f32 * crop.height as f32 / crop.width as f32) / 2.0)
        .round()
        .max(1.0) as usize;

    let mut out = String::new();
    out.push_str(&format!("avatar 1 {cols}x{rows}\n"));
    for (delay_ms, surface) in &composed {
        out.push_str(&format!("f {delay_ms}\n"));
        for row in 0..rows {
            out.push_str(&sample_row(surface, &crop, cols, rows, row, args));
            out.push('\n');
        }
    }
    Ok(out)
}

/// The region of the canvas the subject ever occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Crop {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

/// The union of every frame's subject, so the animation fills the panel
/// instead of centring a small figure in a field of blank rows.
fn bounds<'a>(frames: impl Iterator<Item = &'a Surface>) -> Crop {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    let (mut width, mut height) = (0, 0);
    for surface in frames {
        width = surface.width;
        height = surface.height;
        for y in 0..surface.height {
            for x in 0..surface.width {
                if surface.solid[y * surface.width + x] {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
    }
    // A GIF with nothing in it at all crops to the whole canvas rather than to
    // an inverted rectangle.
    if x0 == usize::MAX || x1 <= x0 || y1 <= y0 {
        return Crop {
            x: 0,
            y: 0,
            width: width.max(1),
            height: height.max(1),
        };
    }
    Crop {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

/// Draw one GIF frame onto the running canvas.
fn compose(canvas: &mut Surface, frame: &gif::Frame<'_>) {
    let (left, top) = (frame.left as usize, frame.top as usize);
    for y in 0..frame.height as usize {
        for x in 0..frame.width as usize {
            let (cx, cy) = (left + x, top + y);
            if cx >= canvas.width || cy >= canvas.height {
                continue;
            }
            let at = (y * frame.width as usize + x) * 4;
            let (r, g, b, a) = (
                frame.buffer[at],
                frame.buffer[at + 1],
                frame.buffer[at + 2],
                frame.buffer[at + 3],
            );
            let target = cy * canvas.width + cx;
            // A transparent pixel in a frame means "leave what was there",
            // which is why the canvas is kept rather than rebuilt.
            if a == 0 {
                continue;
            }
            canvas.lum[target] = luminance(r, g, b);
            canvas.solid[target] = true;
        }
    }
}

/// Rec. 601 luma, which is what the eye actually weighs the channels at.
fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((0.299 * r as f32) + (0.587 * g as f32) + (0.114 * b as f32)).round() as u8
}

/// One row of characters, each averaging the block of pixels under it.
///
/// Averaging rather than sampling one pixel: a paperclip is mostly thin lines,
/// and point sampling drops whichever ones fall between the sample points,
/// which reads as the animation flickering rather than as detail being lost.
fn sample_row(
    canvas: &Surface,
    crop: &Crop,
    cols: usize,
    rows: usize,
    row: usize,
    args: &Args,
) -> String {
    let mut line = String::with_capacity(cols);
    let y0 = crop.y + row * crop.height / rows;
    let y1 = (crop.y + (row + 1) * crop.height / rows).max(y0 + 1);
    for col in 0..cols {
        let x0 = crop.x + col * crop.width / cols;
        let x1 = (crop.x + (col + 1) * crop.width / cols).max(x0 + 1);
        let (mut total, mut counted, mut cells) = (0u32, 0u32, 0u32);
        for y in y0..y1.min(canvas.height) {
            for x in x0..x1.min(canvas.width) {
                cells += 1;
                if canvas.solid[y * canvas.width + x] {
                    total += u32::from(canvas.lum[y * canvas.width + x]);
                    counted += 1;
                }
            }
        }
        // A cell that is mostly not part of the picture is not part of it.
        if counted == 0 || cells == 0 || counted * 2 < cells {
            line.push(' ');
            continue;
        }
        let mut level = (total as f32 / counted as f32) / 255.0;
        if args.invert {
            level = 1.0 - level;
        }
        if level < args.floor {
            line.push(' ');
            continue;
        }
        let step = ((level * (RAMP.len() - 1) as f32).round() as usize).min(RAMP.len() - 1);
        line.push(RAMP[step] as char);
    }
    // Trailing spaces are invisible and would be a third of the file.
    line.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(width: usize, height: usize, lum: u8) -> Surface {
        Surface {
            width,
            height,
            lum: vec![lum; width * height],
            solid: vec![true; width * height],
        }
    }

    /// The whole canvas, which is what an uncropped test wants.
    fn whole(surface: &Surface) -> Crop {
        Crop {
            x: 0,
            y: 0,
            width: surface.width,
            height: surface.height,
        }
    }

    fn args() -> Args {
        Args {
            input: String::new(),
            output: None,
            cols: 4,
            invert: false,
            floor: 0.0,
        }
    }

    /// The whole point of the ramp: brighter is denser.
    fn row(surface: &Surface, args: &Args) -> String {
        sample_row(surface, &whole(surface), 4, 1, 0, args)
    }

    /// Where a row's character sits on the ramp. Comparing the strings
    /// themselves compares their byte values, which is not the ramp's order.
    fn density(row: &str) -> usize {
        RAMP.iter()
            .position(|c| *c as char == row.chars().next().unwrap_or(' '))
            .expect("every character drawn comes from the ramp")
    }

    #[test]
    fn brightness_picks_a_denser_character() {
        let dark = row(&canvas(4, 2, 20), &args());
        let mid = row(&canvas(4, 2, 128), &args());
        let bright = row(&canvas(4, 2, 255), &args());
        assert_eq!(bright, "@@@@");
        assert!(
            density(&dark) < density(&mid) && density(&mid) < density(&bright),
            "{dark:?} {mid:?} {bright:?}"
        );
    }

    /// Transparent pixels are "whatever is behind this", not black. Averaging
    /// them in draws a halo around the subject.
    #[test]
    fn transparency_is_absence_rather_than_darkness() {
        let mut surface = canvas(4, 2, 255);
        // Blank the right half.
        for y in 0..2 {
            for x in 2..4 {
                surface.solid[y * 4 + x] = false;
            }
        }
        assert_eq!(row(&surface, &args()), "@@");
    }

    /// A subject that is dark on a light background needs the ramp the other
    /// way up, or it comes out as a solid block with a hole in it.
    #[test]
    fn inverting_swaps_which_end_is_dense() {
        let mut inverted = args();
        inverted.invert = true;
        assert_eq!(row(&canvas(4, 2, 0), &inverted), "@@@@");
        assert_eq!(row(&canvas(4, 2, 255), &inverted), "");
    }

    /// Near-black cells are compression noise, not shading.
    #[test]
    fn the_floor_blanks_what_is_only_barely_there() {
        let mut floored = args();
        floored.floor = 0.5;
        assert_eq!(row(&canvas(4, 2, 100), &floored), "");
        assert_eq!(row(&canvas(4, 2, 200), &floored).len(), 4);
    }

    /// The crop is the union across every frame. Cropping each frame to its
    /// own subject makes the subject jump around inside the panel.
    #[test]
    fn the_crop_covers_every_frame_and_survives_an_empty_one() {
        let mut first = canvas(8, 8, 200);
        let mut second = canvas(8, 8, 200);
        first.solid.iter_mut().for_each(|s| *s = false);
        second.solid.iter_mut().for_each(|s| *s = false);
        // Two frames, two different corners.
        first.solid[8 + 1] = true;
        second.solid[5 * 8 + 6] = true;
        let crop = bounds([&first, &second].into_iter());
        assert_eq!(
            crop,
            Crop {
                x: 1,
                y: 1,
                width: 6,
                height: 5
            }
        );
        // Nothing anywhere crops to the whole canvas, not to an inverted box.
        let empty = {
            let mut surface = canvas(8, 8, 0);
            surface.solid.iter_mut().for_each(|s| *s = false);
            surface
        };
        assert_eq!(
            bounds([&empty].into_iter()),
            Crop {
                x: 0,
                y: 0,
                width: 8,
                height: 8
            }
        );
    }

    /// A square image on a square grid comes out squashed, because a cell is
    /// twice as tall as it is wide.
    #[test]
    fn the_row_count_accounts_for_the_shape_of_a_character() {
        let rows = |cols: usize, w: usize, h: usize| {
            ((cols as f32 * h as f32 / w as f32) / 2.0).round().max(1.0) as usize
        };
        assert_eq!(rows(40, 100, 100), 20);
        assert_eq!(rows(40, 200, 100), 10);
        assert!(rows(4, 100, 1000) >= 1, "never zero rows");
    }
}
