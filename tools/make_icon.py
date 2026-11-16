# Draws the Sprawl app icon: python tools/make_icon.py assets/sprawl  (writes .ico and .png files)
import sys
from PIL import Image, ImageDraw, ImageChops
out = sys.argv[1]
S = 1024
def grad():
    g = Image.new("RGBA", (S, S))
    stops = [(0.0, (255, 179, 71)), (0.55, (255, 94, 98)), (1.0, (142, 84, 233))]
    px = g.load()
    for y in range(S):
        for x in range(S):
            t = (x + y) / (2 * S - 2)
            for (a, ca), (b, cb) in zip(stops, stops[1:]):
                if a <= t <= b:
                    k = (t - a) / (b - a)
                    px[x, y] = tuple(round(ca[i] + (cb[i] - ca[i]) * k) for i in range(3)) + (255,)
                    break
    return g
G = grad()
def make(detail):
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, S - 1, S - 1), radius=int(S * 0.23), fill=255)
    img.paste(G, (0, 0), mask)
    # skyline: (left, width, height) as fractions; baseline at 0.74
    base = 0.74
    # a low, wide skyline: five blocks of mixed heights, none standing alone
    blds = [(0.13, 0.15, 0.24), (0.27, 0.15, 0.38), (0.41, 0.16, 0.30), (0.56, 0.15, 0.42), (0.70, 0.17, 0.20)]
    city = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(city)
    for l, w, h in blds:
        d.rounded_rectangle((l * S, (base - h) * S, (l + w) * S, base * S), radius=int(S * 0.025), fill=255)
    if detail:
        win = Image.new("L", (S, S), 0)
        dw = ImageDraw.Draw(win)
        for l, w, h in blds:
            cols = 2
            ww = w * 0.22
            for c in range(cols):
                x0 = l + w * (0.2 + c * 0.38)
                y = base - h + 0.06
                r = 0
                while y + 0.045 < base - 0.03:
                    if (r + c + int(l * 100)) % 3 != 0:
                        dw.rectangle((x0 * S, y * S, (x0 + ww) * S, (y + 0.035) * S), fill=255)
                    y += 0.075
                    r += 1
        city = ImageChops.subtract(city, win)
    white = Image.new("RGBA", (S, S), (255, 255, 255, 255))
    img.paste(white, (0, 0), city)
    if detail:
        # the mirror floor: a faded reflection under the baseline
        refl = city.transpose(Image.FLIP_TOP_BOTTOM)
        off = int((2 * base - 1) * S)
        r2 = Image.new("L", (S, S), 0)
        r2.paste(refl, (0, off))
        fade = Image.new("L", (S, S), 0)
        fd = ImageDraw.Draw(fade)
        for y in range(int(base * S), S):
            fd.line((0, y, S, y), fill=max(0, int(110 * (1 - (y - base * S) / (0.16 * S)))))
        r2 = ImageChops.multiply(r2, fade)
        r2 = ImageChops.multiply(r2, mask)
        img.paste(white, (0, 0), r2)
    return img
big, small = make(True), make(False)
big.save(out + ".png")
sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
frames = [(small if s < 40 else big).resize((s, s), Image.LANCZOS) for s in sizes]
frames[-1].save(out + ".ico", sizes=[(s, s) for s in sizes], append_images=frames[:-1])
big.resize((256, 256), Image.LANCZOS).save(out + "-256.png")
print("ok")
