#!/usr/bin/env python3
"""Convert seconds 2..end of a source video into a green Matrix-style ASCII
animation, frame by frame, as a seamless ping-pong loop."""
import os, subprocess, shutil
import numpy as np
from PIL import Image, ImageDraw, ImageFont

SRC = '/home/hec/Downloads/make_a_video_total_black_back.mp4'
FRAMES_DIR = '/tmp/ascii_src_frames'
OUT_DIR = '/tmp/ascii_out_frames'
DEST = '/home/hec/Downloads/ascii_video_loop.mp4'

START_S = 2.0
FONT_SIZE = 16
RAMP = " .:-=+*#%@"

# ---------- probe ----------
fps = float(subprocess.run(['ffprobe', '-v', 'error', '-select_streams', 'v:0',
    '-show_entries', 'stream=r_frame_rate', '-of', 'csv=p=0', SRC],
    capture_output=True, text=True, check=True).stdout.strip().split('/')[0])
fps = 24.0  # source is 24 fps

# ---------- extract source frames from second 2 ----------
shutil.rmtree(FRAMES_DIR, ignore_errors=True)
os.makedirs(FRAMES_DIR)
subprocess.run(['ffmpeg', '-y', '-v', 'error', '-ss', str(START_S), '-i', SRC,
                '-vf', f'fps={int(fps)}', f'{FRAMES_DIR}/s%04d.png'], check=True)
src_frames = sorted(os.listdir(FRAMES_DIR))
n = len(src_frames)
print(f'{n} source frames @ {fps:g} fps')

# ---------- grid setup ----------
probe = Image.open(f'{FRAMES_DIR}/{src_frames[0]}')
img_w, img_h = probe.size
font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf', FONT_SIZE)
cell_w = int(round(font.getlength('M')))
ascent, descent = font.getmetrics()
cell_h = ascent + descent
cols, rows = img_w // cell_w, img_h // cell_h
frame_w, frame_h = cols * cell_w, rows * cell_h
if frame_w % 2: frame_w += 1
if frame_h % 2: frame_h += 1
print(f'grid {cols}x{rows}, cell {cell_w}x{cell_h}, frame {frame_w}x{frame_h}')

# ---------- glyph atlas ----------
atlas = np.zeros((len(RAMP), cell_h, cell_w), dtype=np.float32)
for i, ch in enumerate(RAMP):
    tile = Image.new('L', (cell_w, cell_h), 0)
    ImageDraw.Draw(tile).text((0, 0), ch, font=font, fill=255)
    atlas[i] = np.asarray(tile, dtype=np.float32) / 255.0

gamma = 0.6
lut = np.array([int(255 * (i / 255) ** gamma) for i in range(256)], dtype=np.uint8)

# ping-pong order: 0..n-1 then n-2..1  -> seamless loop
order = list(range(n)) + list(range(n - 2, 0, -1))
period = len(order)

shutil.rmtree(OUT_DIR, ignore_errors=True)
os.makedirs(OUT_DIR)

for pos, fi in enumerate(order):
    img = Image.open(f'{FRAMES_DIR}/{src_frames[fi]}').convert('RGB')

    gray = np.asarray(img.convert('L').resize((cols, rows)), dtype=np.uint8)
    base = lut[gray].astype(np.float32) / 255.0
    char_idx = (base * (len(RAMP) - 1)).astype(np.int32)

    color = np.asarray(img.resize((cols, rows)), dtype=np.int32)
    r, g, b = color[..., 0], color[..., 1], color[..., 2]
    eye_mask = (r > 70) & (r > g * 1.4) & (r > b * 1.4)

    # matrix flavor, deterministic per loop position -> seamless
    frng = np.random.default_rng(pos)
    flicker = 0.9 + 0.2 * frng.random((rows, cols), dtype=np.float32)
    # sparse dim glyphs drifting in the pure-black background
    bg = base < 0.04
    spawn = bg & (frng.random((rows, cols)) < 0.02)
    bg_chars = frng.integers(1, len(RAMP), size=(rows, cols))
    bg_bright = frng.uniform(0.05, 0.25, size=(rows, cols)).astype(np.float32)

    bright = base * flicker
    bright = np.where(spawn, bg_bright, bright)
    chars = np.where(spawn, bg_chars, char_idx)
    bright = np.where(eye_mask, 1.0, bright)
    chars = np.where(eye_mask, RAMP.index('@'), chars)

    frame = (atlas[chars] * bright[..., None, None])
    frame = frame.transpose(0, 2, 1, 3).reshape(rows * cell_h, cols * cell_w)
    green = np.clip(frame * 255, 0, 255)

    rgb = np.zeros((*green.shape, 3), dtype=np.uint8)
    rgb[..., 1] = green.astype(np.uint8)
    hot = np.clip(green.astype(np.float32) - 190, 0, 65) / 65  # near-white hotspots
    rgb[..., 0] = (hot * 120).astype(np.uint8)
    rgb[..., 2] = (hot * 120).astype(np.uint8)

    out = Image.new('RGB', (frame_w, frame_h), (0, 0, 0))
    out.paste(Image.fromarray(rgb), (0, 0))
    out.save(f'{OUT_DIR}/f{pos:04d}.png')

print(f'{period} output frames, encoding...')
subprocess.run(['ffmpeg', '-y', '-framerate', str(int(fps)), '-i', f'{OUT_DIR}/f%04d.png',
                '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-crf', '18',
                '-movflags', '+faststart', DEST], check=True, capture_output=True)
shutil.rmtree(FRAMES_DIR)
shutil.rmtree(OUT_DIR)
print('done:', DEST, f'({period / fps:.1f}s loop)')
