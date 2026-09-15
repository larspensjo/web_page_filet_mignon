"""Render Harvester identity assets from the SVG sources.

Rasterizes harvester-icon.svg with headless Microsoft Edge (the only SVG
rasterizer guaranteed on a Windows 11 dev box), then uses Pillow to produce
the PNG ladder and the multi-size .ico consumed by crates/harvester_ui.

Usage:  python assets/identity/source/render.py
"""
import os, shutil, subprocess, sys, tempfile
from pathlib import Path
from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
GENERATED = HERE.parent / "generated"
UI_ICONS = ROOT / "crates" / "harvester_ui" / "icons"
MASTER = 1024
SIZES = [16, 24, 32, 48, 64, 128, 256, 512]
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]

def find_edge():
    for p in (r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
              r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"):
        if os.path.exists(p):
            return p
    return shutil.which("msedge") or shutil.which("chrome")

def rasterize(svg: Path, out: Path):
    edge = find_edge()
    if not edge:
        sys.exit("No headless browser found (Edge/Chrome) to rasterize the SVG.")
    with tempfile.TemporaryDirectory() as tmp:
        page = Path(tmp) / "page.html"
        page.write_text(f'<!doctype html><style>html,body{{margin:0;background:transparent}}'
                        f'img{{display:block;width:{MASTER}px;height:{MASTER}px}}</style>'
                        f'<img src="{svg.as_uri()}">', encoding="utf-8")
        subprocess.run([edge, "--headless=new", "--disable-gpu", "--hide-scrollbars",
                        "--default-background-color=00000000",
                        f"--window-size={MASTER},{MASTER}", f"--screenshot={out}", page.as_uri()],
                       check=True, capture_output=True)

def main():
    GENERATED.mkdir(parents=True, exist_ok=True)
    UI_ICONS.mkdir(parents=True, exist_ok=True)
    master = GENERATED / "harvester-icon-1024.png"
    rasterize(HERE / "harvester-icon.svg", master)
    img = Image.open(master).convert("RGBA")
    assert img.size == (MASTER, MASTER), img.size
    ladder = {}
    for s in SIZES:
        ladder[s] = img.resize((s, s), Image.LANCZOS)
        ladder[s].save(GENERATED / f"harvester-icon-{s}.png")
    ico = GENERATED / "harvester.ico"
    ladder[256].save(ico, format="ICO", sizes=[(s, s) for s in ICO_SIZES],
                     append_images=[ladder[s] for s in ICO_SIZES if s != 256])
    shutil.copyfile(ico, UI_ICONS / "icon.ico")
    for s in (32, 128):
        shutil.copyfile(GENERATED / f"harvester-icon-{s}.png", UI_ICONS / f"{s}x{s}.png")
    shutil.copyfile(GENERATED / "harvester-icon-256.png", UI_ICONS / "128x128@2x.png")
    shutil.copyfile(GENERATED / "harvester-icon-512.png", UI_ICONS / "icon.png")
    print("rendered", sorted(p.name for p in GENERATED.iterdir()))

if __name__ == "__main__":
    main()
