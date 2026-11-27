#!/usr/bin/env python3
"""Make the README pictures in docs/ from the sources in docs/src/.

usage: python3 docs/src/make.py [BINARY]

BINARY is a repo-status built from this tree (default: a copy of target/release/repo-status);
the window pictures are taken from `repo-status --demo`, so they show made-up repos.
Needs: firefox (for the drawn pictures), gtk4-broadwayd (for the window), Pillow, optipng (optional). Nothing appears on your desktop.
window-windows.png is not made here: it is the picture the CI `windows` job saves
(artifact repo-status-windows, file window-demo.png); round its corners with:
    python3 docs/src/make.py --round FILE.png docs/window-windows.png
"""
import pathlib, shutil, subprocess, sys, tempfile
from PIL import Image, ImageDraw

SRC = pathlib.Path(__file__).resolve().parent
DOCS = SRC.parent
BORDER = (0x33, 0x39, 0x41, 255)

def rounded(im, radius=28, border=True):
    """Round the corners (transparent outside) so a picture sits well on a light or a dark page."""
    im = im.convert("RGBA")
    big = 4
    mask = Image.new("L", (im.width * big, im.height * big), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, mask.width - 1, mask.height - 1), radius * big, fill=255)
    im.putalpha(mask.resize(im.size, Image.LANCZOS))
    if border:
        edge = Image.new("RGBA", (im.width * big, im.height * big), (0, 0, 0, 0))
        ImageDraw.Draw(edge).rounded_rectangle(
            (big, big, edge.width - 1 - big, edge.height - 1 - big), radius * big, outline=BORDER, width=2 * big)
        im.alpha_composite(edge.resize(im.size, Image.LANCZOS))
    return im

def save(im, name):
    out = DOCS / name
    im.save(out)
    if shutil.which("optipng"):
        subprocess.run(["optipng", "-quiet", "-o2", str(out)], check=False)
    print(f"{name}: {im.width}x{im.height}, {out.stat().st_size // 1024} KB")

def page(html, width, height, tmp):
    """A headless Firefox screenshot of docs/src/HTML at width x height pixels."""
    shot = tmp / (html + ".png")
    subprocess.run(
        ["firefox", "--headless", "--no-remote", "--profile", str(tmp / "ff"), f"--window-size={width},{height}",
         "--screenshot", str(shot), (SRC / html).as_uri()],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
    return Image.open(shot).crop((0, 0, width, height))

def main():
    if sys.argv[1:2] == ["--round"]:
        rounded(Image.open(sys.argv[2])).save(sys.argv[3])
        return
    # work files under target/ and /tmp: a Firefox from a snap sees your home folder but not the
    # real /tmp, and the window binary needs a path without spaces
    work = DOCS.parent / "target"
    work.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="docs-", dir=work) as tmp, tempfile.TemporaryDirectory() as bin_dir:
        tmp = pathlib.Path(tmp)
        (tmp / "ff").mkdir()
        binary = pathlib.Path(bin_dir) / "repo-status"
        shutil.copy(sys.argv[1] if len(sys.argv) > 1 else DOCS.parent / "target/release/repo-status", binary)
        snap = lambda name, tab, size: subprocess.run(
            [str(SRC / "snap-gtk.sh"), str(binary), str(tmp / name), str(tab), size], check=True)
        snap("w0.png", 0, "940x940")
        snap("w1.png", 1, "940x770")
        w0 = Image.open(tmp / "w0.png")
        save(rounded(w0), "window.png")
        save(rounded(Image.open(tmp / "w1.png")), "window-tabs.png")
        # the second card of the first tab (you/tiny-parser), for "how to read a card"
        card = w0.crop(CARD_BOX)
        card.save(SRC / "card-crop.png")
        save(rounded(page("banner.html", 1720, 480, tmp)), "banner.png")
        save(rounded(page("flow.html", 1720, 600, tmp)), "flow.png")
        save(rounded(page("card.html", 1720, 1040, tmp)), "card.png")
        (SRC / "card-crop.png").unlink()
        social = page("social.html", 1280, 640, tmp).convert("RGB")
        save(social, "social-preview.png")

# left, top, right, bottom of that card in the 2x window picture; check card.png after a layout change
CARD_BOX = (32, 636, 1848, 1182)

if __name__ == "__main__":
    main()
