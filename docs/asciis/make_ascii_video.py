#!/usr/bin/env python3
"""Render an image as a Matrix-style ASCII animation video."""
import os, subprocess, shutil
import numpy as np
from PIL import Image, ImageDraw, ImageFont

SRC = '/home/hec/Downloads/9ac2f8f8-36a4-4c16-a412-1a002a593b3a.png'
OUT_DIR = '/home/hec/Downloads/ascii_frames'
FULL_MP4 = '/home/hec/Downloads/ascii_matrix.mp4'
LOOP_MP4 = '/home/hec/Downloads/ascii_matrix_loop.mp4'

FPS = 24
INTRO_S = 2          # fade-in from black
LOOP_S = 10          # seamlessly looping segment after the intro
FONT_SIZE = 16
RAMP = " .:-=+*#%@"

# ---------- setup ----------
img = Image.open(SRC).convert('RGB')
img_w, img_h = img.size

font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf', FONT_SIZE)
cell_w = int(round(font.getlength('M')))
ascent, descent = font.getmetrics()
cell_h = ascent + descent

cols = img_w // cell_w
rows = img_h // cell_h
frame_w, frame_h = cols * cell_w, rows * cell_h
if frame_w % 2: frame_w += 1   # keep even for yuv420p
if frame_h % 2: frame_h += 1
print(f'grid {cols}x{rows}, cell {cell_w}x{cell_h}, frame {frame_w}x{frame_h}')

# ---------- static grids from the image ----------
gray = img.convert('L').resize((cols, rows))
gamma = 0.55
lut = [int(255 * (i / 255) ** gamma) for i in range(256)]
gray = gray.point(lut)
base = np.asarray(gray, dtype=np.float32) / 255.0            # (rows, cols)
char_idx = (base * (len(RAMP) - 1)).astype(np.int32)

color = np.asarray(img.resize((cols, rows)), dtype=np.int32)
r, g, b = color[..., 0], color[..., 1], color[..., 2]
eye_mask = (r > 80) & (r > g * 1.5) & (r > b * 1.5)

# ---------- glyph atlas ----------
atlas = np.zeros((len(RAMP), cell_h, cell_w), dtype=np.float32)
for i, ch in enumerate(RAMP):
    tile = Image.new('L', (cell_w, cell_h), 0)
    ImageDraw.Draw(tile).text((0, 0), ch, font=font, fill=255)
    atlas[i] = np.asarray(tile, dtype=np.float32) / 255.0
eye_glyph = atlas[RAMP.index('@')]

# ---------- rain setup (periodic over LOOP_S -> seamless loop) ----------
rng = np.random.default_rng(42)
SPAN = rows + 24
TRAIL = 18
passes = rng.integers(1, 4, size=cols)                  # full descents per loop
speeds = passes * SPAN / LOOP_S                         # cells per second
offsets = rng.uniform(0, SPAN, size=cols)
ys = np.arange(rows, dtype=np.float32)[:, None]         # (rows, 1)

n_frames = int((INTRO_S + LOOP_S) * FPS)
loop_frames = int(LOOP_S * FPS)
os.makedirs(OUT_DIR, exist_ok=True)

for f in range(n_frames):
    t = f / FPS
    fade = min(1.0, t / INTRO_S)

    # rain brightness boost
    head = (speeds * t + offsets) % SPAN                # (cols,)
    d = (head[None, :] - ys) % SPAN                     # (rows, cols)
    boost = np.where(d < TRAIL, np.exp(-d / 6.0), 0.0).astype(np.float32)

    # inside the rain, scramble chars (deterministic per loop position)
    idx = f % loop_frames
    frng = np.random.default_rng(idx)
    scramble = (boost > 0.25) & (frng.random((rows, cols)) < 0.6)
    chars = np.where(scramble, frng.integers(1, len(RAMP), size=(rows, cols)), char_idx)

    bright = np.clip(base + 0.95 * boost, 0, 1) * fade

    # eyes: pulsing bright glow (period 5s divides LOOP_S -> seamless)
    pulse = 0.8 + 0.2 * np.sin(2 * np.pi * t / 5)
    bright = np.where(eye_mask, fade * pulse, bright)
    chars = np.where(eye_mask, RAMP.index('@'), chars)

    # compose green channel
    frame = (atlas[chars] * bright[..., None, None])    # (rows, cols, ch, cw)
    frame = frame.transpose(0, 2, 1, 3).reshape(rows * cell_h, cols * cell_w)

    green = np.clip(frame * 255, 0, 255)
    rgb = np.zeros((*green.shape, 3), dtype=np.uint8)
    rgb[..., 1] = green.astype(np.uint8)
    # near-white rain heads
    head_glow = (boost > 0.8).astype(np.float32) * fade
    hg = np.repeat(np.repeat(head_glow, cell_h, axis=0), cell_w, axis=1)
    rgb[..., 0] = np.clip(green * hg * 0.85, 0, 255).astype(np.uint8)
    rgb[..., 2] = np.clip(green * hg * 0.85, 0, 255).astype(np.uint8)

    out = Image.new('RGB', (frame_w, frame_h), (0, 0, 0))
    out.paste(Image.fromarray(rgb), (0, 0))
    out.save(f'{OUT_DIR}/f{f:04d}.png')

print('frames done, encoding...')

def encode(first, last, dest):
    subprocess.run([
        'ffmpeg', '-y', '-framerate', str(FPS),
        '-start_number', str(first), '-i', f'{OUT_DIR}/f%04d.png',
        '-frames:v', str(last - first + 1),
        '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-crf', '18', '-movflags', '+faststart',
        dest], check=True, capture_output=True)

encode(0, n_frames - 1, FULL_MP4)                       # intro + one loop
encode(INTRO_S * FPS, n_frames - 1, LOOP_MP4)           # seamless loop only
shutil.rmtree(OUT_DIR)
print('done:', FULL_MP4, LOOP_MP4)
