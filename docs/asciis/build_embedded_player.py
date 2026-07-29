#!/usr/bin/env python3
"""Build a single self-contained player script with all ASCII frames embedded."""
import base64, gzip, os, shutil, stat, subprocess
import numpy as np
from PIL import Image

SRC = '/home/hec/Downloads/make_a_video_total_black_back.mp4'
FRAMES_DIR = '/tmp/embed_src_frames'
DEST = '/home/hec/Downloads/ascii_loop.py'

START_S = 2.0
FPS = 24
COLS, ROWS = 128, 37
RAMP = " .:-=+*#%@"

shutil.rmtree(FRAMES_DIR, ignore_errors=True)
os.makedirs(FRAMES_DIR)
subprocess.run(['ffmpeg', '-y', '-v', 'error', '-ss', str(START_S), '-i', SRC,
                '-vf', f'fps={FPS}', f'{FRAMES_DIR}/s%04d.png'], check=True)
names = sorted(os.listdir(FRAMES_DIR))
print(f'{len(names)} source frames')

gamma = 0.6
lut = np.array([int(255 * (i / 255) ** gamma) for i in range(256)], dtype=np.uint8)
n = len(names)
order = list(range(n)) + list(range(n - 2, 0, -1))   # ping-pong

frames = []
for pos, fi in enumerate(order):
    img = Image.open(f'{FRAMES_DIR}/{names[fi]}').convert('RGB')
    gray = np.asarray(img.convert('L').resize((COLS, ROWS)), dtype=np.uint8)
    base = lut[gray].astype(np.float32) / 255.0
    chars = (base * (len(RAMP) - 1)).astype(np.int32)

    color = np.asarray(img.resize((COLS, ROWS)), dtype=np.int32)
    r, g, b = color[..., 0], color[..., 1], color[..., 2]
    eye = (r > 70) & (r > g * 1.4) & (r > b * 1.4)

    frng = np.random.default_rng(pos)
    spawn = (base < 0.04) & (frng.random((ROWS, COLS)) < 0.02)
    chars = np.where(spawn, frng.integers(1, len(RAMP) - 3, size=(ROWS, COLS)), chars)
    chars = np.where(eye, RAMP.index('='), chars)

    rows = [''.join(RAMP[c] for c in row).rstrip() for row in chars]
    frames.append('\n'.join(rows))

shutil.rmtree(FRAMES_DIR)
blob = base64.b85encode(gzip.compress('\x00'.join(frames).encode(), 9)).decode()
print(f'{len(frames)} frames embedded, blob {len(blob)//1024} KiB')

player = '''#!/usr/bin/env python3
"""Self-contained ASCII animation (seconds 2-10, Matrix green, seamless loop).
All frames are embedded below - no external files needed. Ctrl+C to quit."""
import base64, gzip, os, shutil, sys, time

FPS = 24
COLS, ROWS = 128, 37
BLOB = """
__BLOB__
"""

GREEN = "\\x1b[38;5;46m"
RESET = "\\x1b[0m"

frames = gzip.decompress(base64.b85decode("".join(BLOB.split()))).decode().split("\\x00")

def fit(frame, tw, th):
    """Downsample by skipping chars/rows if the terminal is too small."""
    lines = frame.split("\\n")
    if tw >= COLS and th >= ROWS:
        return frame
    sy = max(1, -(-ROWS // th)) if th < ROWS else 1
    sx = max(1, -(-COLS // tw)) if tw < COLS else 1
    return "\\n".join(l[::sx][:tw] for l in lines[::sy][:th])

def main():
    ts = shutil.get_terminal_size()
    out = sys.stdout
    out.write("\\x1b[?25l\\x1b[2J")          # hide cursor, clear
    delay = 1.0 / FPS
    i, n = 0, len(frames)
    try:
        while True:
            t0 = time.monotonic()
            frame = frames[i]
            out.write("\\x1b[H" + GREEN + fit(frame, ts.columns, ts.lines - 1))
            out.flush()
            i = (i + 1) % n
            dt = time.monotonic() - t0
            if dt < delay:
                time.sleep(delay - dt)
    except KeyboardInterrupt:
        pass
    finally:
        out.write(RESET + "\\x1b[?25h\\x1b[2J\\x1b[H")
        out.flush()

if __name__ == "__main__":
    main()
'''

blob_lines = '\\n'.join(blob[i:i + 100] for i in range(0, len(blob), 100))
script = player.replace('__BLOB__', blob_lines)
with open(DEST, 'w') as f:
    f.write(script)
os.chmod(DEST, os.stat(DEST).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
print('done:', DEST, f'({os.path.getsize(DEST)//1024} KiB)')
