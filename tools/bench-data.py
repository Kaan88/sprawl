"""Write a large synthetic scan as a Sprawl/WizTree CSV, for the benchmark (tools/bench.ps1).

Nothing is read from or written to any drive but the output file: the paths in the CSV are
made up, under a drive letter of their own (Q: by default), shaped like a well-used Windows
drive: Windows, Program Files, ProgramData, a user profile with browser and package caches,
photos, music and video, and many code projects with node_modules, .git, Rust target and
Python virtual environments. About 1.5 million files at scale 1.0. The same seed gives the
same file.

    python tools/bench-data.py [--out target/bench/large.csv] [--scale 1.0] [--root Q:]
"""

import argparse
import math
import os
import random

EPOCH_DAY = 20_400  # days since 1970: 2025-11-07, a fixed "now" so the file never changes


class Writer:
    def __init__(self, out, rng):
        self.out = out
        self.rng = rng
        self.files = 0
        self.folders = 0
        self.bytes = 0

    def when(self, mean_days=220.0):
        r = self.rng
        days = min(r.expovariate(1.0 / mean_days), 3000)
        t = (EPOCH_DAY - days) * 86400 + r.randrange(86400)
        y, mo, d, h, mi, s = to_civil(int(t))
        return f"{y:04d}/{mo:02d}/{d:02d} {h:02d}:{mi:02d}:{s:02d}"

    def folder(self, path):
        self.out.write(f'"{path}\\",0,0,{self.when()},16,0,0\r\n')
        self.folders += 1

    def file(self, path, size, mean_days=220.0, attrs=32):
        alloc = 0 if size == 0 else (size + 4095) // 4096 * 4096
        self.out.write(f'"{path}",{size},{alloc},{self.when(mean_days)},{attrs},0,0\r\n')
        self.files += 1
        self.bytes += size


def to_civil(t):
    """Seconds since 1970 (UTC) as (year, month, day, hour, minute, second)."""
    days, rem = divmod(t, 86400)
    h, rem = divmod(rem, 3600)
    mi, s = divmod(rem, 60)
    z = days + 719468
    era = z // 146097
    doe = z - era * 146097
    yoe = (doe - doe // 1460 + doe // 36524 - doe // 146096) // 365
    y = yoe + era * 400
    doy = doe - (365 * yoe + yoe // 4 - yoe // 100)
    mp = (5 * doy + 2) // 153
    d = doy - (153 * mp + 2) // 5 + 1
    m = mp + 3 if mp < 10 else mp - 9
    return (y + (m <= 2), m, d, h, mi, s)


# typical sizes: (median bytes, spread as the log-normal sigma)
SIZES = {
    "dll": (180_000, 1.6), "exe": (400_000, 1.8), "sys": (60_000, 1.2), "mui": (20_000, 1.0),
    "inf": (8_000, 1.0), "cat": (12_000, 0.8), "manifest": (2_000, 0.8), "msi": (8_000_000, 1.5),
    "msp": (40_000_000, 1.2), "cab": (30_000_000, 1.4), "ttf": (300_000, 1.0), "js": (6_000, 1.6),
    "mjs": (5_000, 1.4), "cjs": (5_000, 1.4), "ts": (4_000, 1.3), "map": (12_000, 1.6),
    "json": (1_500, 1.5), "md": (4_000, 1.2), "txt": (2_000, 1.5), "png": (40_000, 1.8),
    "jpg": (2_500_000, 0.6), "heic": (1_800_000, 0.5), "mp4": (120_000_000, 1.0),
    "mov": (400_000_000, 1.0), "mp3": (6_000_000, 0.4), "flac": (30_000_000, 0.4),
    "pdf": (900_000, 1.6), "docx": (60_000, 1.3), "xlsx": (40_000, 1.3), "zip": (6_000_000, 1.6),
    "iso": (4_500_000_000, 0.4), "rs": (8_000, 1.3), "rlib": (2_000_000, 1.8), "rmeta": (400_000, 1.6),
    "o": (300_000, 1.8), "d": (2_000, 1.0), "pdb": (6_000_000, 1.2), "py": (9_000, 1.4),
    "pyc": (12_000, 1.4), "pyd": (500_000, 1.5), "cs": (6_000, 1.2), "html": (18_000, 1.2),
    "log": (200_000, 2.0), "tmp": (100_000, 2.5), "etl": (2_000_000, 1.0), "dat": (1_000_000, 2.0),
    "pak": (4_000_000, 1.2), "gamepak": (2_000_000_000, 0.8), "xml": (6_000, 1.4), "crate": (150_000, 1.4), "nupkg": (800_000, 1.5),
    "lock": (100_000, 1.0), "wer": (20_000, 1.0), "": (50_000, 2.0), "idx": (1_000_000, 1.5),
    "pack": (50_000_000, 2.0), "bin": (150_000, 1.6), "sqlite": (5_000_000, 1.5), "db": (2_000_000, 2.0),
    "svg": (4_000, 1.2), "css": (8_000, 1.4), "woff2": (60_000, 0.8), "h": (10_000, 1.4), "c": (20_000, 1.4),
    "ldb": (2_000_000, 1.0), "jar": (2_000_000, 1.5), "class": (4_000, 1.2),
}


def size_of(rng, ext):
    med, sigma = SIZES.get(ext, (50_000, 1.5))
    return max(0, int(med * math.exp(rng.gauss(0.0, sigma))))


SYL = ["ka", "lo", "mi", "ne", "ra", "to", "vi", "zu", "be", "co", "da", "fe", "gi", "ho", "ju", "pa", "qui", "se", "ty", "wo",
       "ex", "an", "or", "el", "is", "un", "ar", "on", "ix", "ly"]


def word(rng, lo=2, hi=4):
    return "".join(rng.choice(SYL) for _ in range(rng.randint(lo, hi)))


def pick(rng, weighted):
    """weighted: list of (value, weight)"""
    total = sum(w for _, w in weighted)
    x = rng.random() * total
    for v, w in weighted:
        x -= w
        if x <= 0:
            return v
    return weighted[-1][0]


def n(scale, k):
    return max(1, int(k * scale))


def flat(w, rng, path, count, exts, mean_days=220.0, name=None):
    """A folder with `count` files of these extensions."""
    w.folder(path)
    seen = set()
    for i in range(count):
        ext = pick(rng, exts)
        stem = name(i) if name else word(rng, 2, 5)
        nm = f"{stem}.{ext}" if ext else stem
        if nm in seen:
            nm = f"{stem}{i}.{ext}" if ext else f"{stem}{i}"
        seen.add(nm)
        w.file(f"{path}\\{nm}", size_of(rng, ext), mean_days)


def tree(w, rng, path, files, exts, depth, fanout=(2, 6), mean_days=220.0):
    """A folder tree holding about `files` files, `depth` levels deep at most."""
    if depth <= 0 or files < 12:
        flat(w, rng, path, max(files, 0), exts, mean_days)
        return
    here = max(1, int(files * rng.uniform(0.1, 0.35)))
    flat(w, rng, path, here, exts, mean_days)
    rest = files - here
    k = rng.randint(*fanout)
    weights = [rng.random() ** 2 + 0.05 for _ in range(k)]
    total = sum(weights)
    names = set()
    for wt in weights:
        sub = word(rng)
        while sub in names:
            sub = word(rng)
        names.add(sub)
        tree(w, rng, f"{path}\\{sub}", int(rest * wt / total), exts, depth - 1, fanout, mean_days)


def windows(w, rng, root, s):
    p = f"{root}\\Windows"
    w.folder(p)
    sys_exts = [("dll", 60), ("exe", 8), ("sys", 4), ("dat", 2), ("xml", 3), ("txt", 1), ("png", 2)]
    flat(w, rng, f"{p}\\System32", n(s, 18_000), sys_exts, 500)
    tree(w, rng, f"{p}\\System32\\en-US", n(s, 4_000), [("mui", 90), ("xml", 10)], 2, mean_days=500)
    w.folder(f"{p}\\System32\\DriverStore")
    w.folder(f"{p}\\System32\\DriverStore\\FileRepository")
    for i in range(n(s, 1_500)):
        flat(w, rng, f"{p}\\System32\\DriverStore\\FileRepository\\{word(rng)}{i}.inf_amd64_{rng.getrandbits(64):016x}",
             rng.randint(3, 14), [("inf", 2), ("sys", 3), ("cat", 1), ("dll", 3), ("bin", 1)], 600)
    flat(w, rng, f"{p}\\SysWOW64", n(s, 12_000), sys_exts, 500)
    w.folder(f"{p}\\WinSxS")
    for i in range(n(s, 20_000)):
        arch = rng.choice(["amd64", "x86", "wow64", "msil"])
        flat(w, rng, f"{p}\\WinSxS\\{arch}_microsoft-windows-{word(rng)}-{word(rng)}_31bf3856ad364e35_10.0.26100.{rng.randint(1, 4000)}_none_{rng.getrandbits(64):016x}",
             rng.randint(1, 7), [("dll", 6), ("exe", 1), ("mui", 2), ("manifest", 1)], 400)
    flat(w, rng, f"{p}\\WinSxS\\Manifests", n(s, 30_000), [("manifest", 1)], 400)
    flat(w, rng, f"{p}\\Installer", n(s, 1_200), [("msi", 3), ("msp", 2)], 800)
    tree(w, rng, f"{p}\\assembly\\NativeImages_v4.0.30319_64", n(s, 10_000), [("dll", 9), ("ini", 1)], 3)
    flat(w, rng, f"{p}\\Fonts", n(s, 1_200), [("ttf", 8), ("fon", 1)], 900)
    tree(w, rng, f"{p}\\servicing\\LCU", n(s, 8_000), [("cab", 1), ("manifest", 5), ("mum", 3), ("cat", 2)], 4)
    flat(w, rng, f"{p}\\Logs\\CBS", n(s, 300), [("log", 5), ("cab", 1)], 30)
    flat(w, rng, f"{p}\\Temp", n(s, 900), [("tmp", 5), ("log", 3), ("etl", 1)], 20)


APP_EXTS = [("dll", 30), ("exe", 3), ("pak", 1), ("dat", 4), ("json", 6), ("png", 12), ("xml", 6), ("js", 10), ("css", 3),
            ("html", 3), ("svg", 5), ("txt", 2), ("woff2", 1), ("jar", 2), ("class", 6), ("h", 3), ("lib", 1)]


def program_files(w, rng, root, s):
    for pf, apps in (("Program Files", 55), ("Program Files (x86)", 25)):
        w.folder(f"{root}\\{pf}")
        for i in range(apps):
            name = f"{word(rng, 2, 3).capitalize()} {word(rng, 2, 3).capitalize()}"
            tree(w, rng, f"{root}\\{pf}\\{name}", n(s, int(rng.choice([80, 300, 900, 2500, 5000]))), APP_EXTS, rng.randint(2, 6), mean_days=300)


def games(w, rng, root, s):
    p = f"{root}\\Games"
    w.folder(p)
    for i in range(10):
        g = f"{p}\\{word(rng, 2, 3).capitalize()} {word(rng, 1, 2).capitalize()}"
        w.folder(g)
        for k in range(rng.randint(3, 12)):
            w.file(f"{g}\\pakchunk{k}-WindowsNoEditor.pak", size_of(rng, "gamepak"), 120)
        tree(w, rng, f"{g}\\Engine", n(s, 1_500), [("dll", 5), ("ini", 3), ("ufont", 1), ("txt", 1), ("json", 2)], 4, mean_days=120)


def program_data(w, rng, root, s):
    p = f"{root}\\ProgramData"
    w.folder(p)
    w.folder(f"{p}\\Package Cache")
    for i in range(n(s, 400)):
        flat(w, rng, f"{p}\\Package Cache\\{{{rng.getrandbits(128):032X}}}", rng.randint(1, 3), [("msi", 3), ("cab", 2), ("exe", 1)], 500)
    flat(w, rng, f"{p}\\Microsoft\\Windows\\WER\\ReportArchive", n(s, 2_000), [("wer", 5), ("dmp", 1), ("xml", 2)], 90)
    tree(w, rng, f"{p}\\Microsoft\\Windows Defender\\Definition Updates", n(s, 600), [("vdm", 3), ("dll", 2), ("dat", 1)], 2, mean_days=10)
    tree(w, rng, f"{p}\\{word(rng).capitalize()}", n(s, 12_000), APP_EXTS, 5)


def git(w, rng, path, s, objects):
    w.folder(path)
    for f in ("HEAD", "config", "description", "index", "packed-refs", "FETCH_HEAD", "ORIG_HEAD"):
        w.file(f"{path}\\{f}", rng.randint(20, 400_000), 30)
    w.folder(f"{path}\\objects")
    loose = n(s, objects)
    dirs = {}
    for _ in range(loose):
        d = f"{rng.getrandbits(8):02x}"
        dirs.setdefault(d, []).append(f"{rng.getrandbits(152):038x}")
    for d in sorted(dirs):
        w.folder(f"{path}\\objects\\{d}")
        for o in dirs[d]:
            w.file(f"{path}\\objects\\{d}\\{o}", size_of(rng, "") // 10, 60)
    w.folder(f"{path}\\objects\\pack")
    for _ in range(rng.randint(1, 6)):
        h = f"{rng.getrandbits(160):040x}"
        w.file(f"{path}\\objects\\pack\\pack-{h}.pack", size_of(rng, "pack"), 60)
        w.file(f"{path}\\objects\\pack\\pack-{h}.idx", size_of(rng, "idx"), 60)
    tree(w, rng, f"{path}\\refs", rng.randint(10, 120), [("", 1)], 3, mean_days=30)
    tree(w, rng, f"{path}\\logs", rng.randint(5, 60), [("", 1)], 3, mean_days=30)


JS_EXTS = [("js", 40), ("mjs", 4), ("cjs", 4), ("ts", 18), ("map", 14), ("json", 8), ("md", 4), ("txt", 1), ("css", 2), ("svg", 1)]


def node_modules(w, rng, path, files, depth=0):
    w.folder(path)
    left = files
    while left > 0:
        scoped = rng.random() < 0.15
        name = f"@{word(rng)}\\{word(rng)}" if scoped else f"{word(rng)}-{word(rng)}" if rng.random() < 0.4 else word(rng)
        pkg = f"{path}\\{name}"
        if scoped:
            w.folder(pkg.rsplit("\\", 1)[0])
        size = min(left, int(rng.choice([4, 8, 15, 30, 60, 120, 400, 1200])))
        w.folder(pkg)
        for f, e in (("package", "json"), ("README", "md"), ("LICENSE", "")):
            w.file(f"{pkg}\\{f}.{e}" if e else f"{pkg}\\{f}", size_of(rng, e or "txt"), 300)
        inner = max(0, size - 3)
        if inner > 0:
            tree(w, rng, f"{pkg}\\{rng.choice(['lib', 'dist', 'build', 'src', 'esm'])}", inner, JS_EXTS, 3, (1, 4), 300)
        left -= size + 3
        if depth < 2 and rng.random() < 0.06 and left > 50:
            nested = min(left, rng.randint(20, 400))
            node_modules(w, rng, f"{pkg}\\node_modules", nested, depth + 1)
            left -= nested


def js_project(w, rng, path, s):
    w.folder(path)
    for f, e in (("package", "json"), ("package-lock", "json"), ("README", "md"), ("tsconfig", "json"), (".gitignore", "")):
        w.file(f"{path}\\{f}.{e}" if e else f"{path}\\{f}", size_of(rng, e or "txt"), 20)
    tree(w, rng, f"{path}\\src", n(s, rng.randint(200, 800)), [("ts", 50), ("tsx", 20), ("css", 8), ("json", 5), ("svg", 5), ("md", 2)], 5, mean_days=40)
    node_modules(w, rng, f"{path}\\node_modules", n(s, rng.choice([8_000, 15_000, 22_000, 35_000])))
    if rng.random() < 0.6:
        tree(w, rng, f"{path}\\dist", n(s, rng.randint(50, 600)), [("js", 5), ("map", 5), ("css", 1), ("html", 1)], 3, mean_days=10)
    git(w, rng, f"{path}\\.git", s, rng.randint(1_000, 6_000))


def rust_project(w, rng, path, s):
    w.folder(path)
    for f in ("Cargo.toml", "Cargo.lock", "README.md", ".gitignore"):
        w.file(f"{path}\\{f}", rng.randint(200, 90_000), 20)
    tree(w, rng, f"{path}\\src", n(s, rng.randint(50, 300)), [("rs", 1)], 4, mean_days=30)
    t = f"{path}\\target"
    w.folder(t)
    for prof in ("debug", "release"):
        if prof == "release" and rng.random() < 0.5:
            continue
        w.folder(f"{t}\\{prof}")
        flat(w, rng, f"{t}\\{prof}\\deps", n(s, rng.randint(1_500, 4_000)), [("rlib", 3), ("rmeta", 3), ("d", 3), ("o", 2), ("pdb", 1), ("dll", 1), ("exe", 1)], 15)
        tree(w, rng, f"{t}\\{prof}\\build", n(s, rng.randint(500, 2_000)), [("o", 2), ("d", 1), ("rs", 1), ("txt", 1), ("exe", 1)], 3, mean_days=15)
        tree(w, rng, f"{t}\\{prof}\\incremental", n(s, rng.randint(1_000, 5_000)), [("bin", 3), ("o", 1), ("lock", 1)], 3, mean_days=5)
    git(w, rng, f"{path}\\.git", s, rng.randint(500, 3_000))


def python_project(w, rng, path, s):
    w.folder(path)
    tree(w, rng, f"{path}\\{word(rng)}", n(s, rng.randint(40, 300)), [("py", 1)], 3, mean_days=30)
    sp = f"{path}\\.venv\\Lib\\site-packages"
    w.folder(f"{path}\\.venv")
    w.folder(f"{path}\\.venv\\Lib")
    tree(w, rng, sp, n(s, rng.randint(6_000, 20_000)), [("py", 10), ("pyc", 8), ("pyd", 1), ("txt", 1), ("json", 1), ("dll", 1)], 5, (3, 12), 200)
    flat(w, rng, f"{path}\\.venv\\Scripts", 30, [("exe", 3), ("py", 1)], 200)
    git(w, rng, f"{path}\\.git", s, rng.randint(300, 2_000))


def dotnet_project(w, rng, path, s):
    w.folder(path)
    tree(w, rng, f"{path}\\src", n(s, rng.randint(100, 500)), [("cs", 8), ("xml", 1), ("json", 1)], 4, mean_days=30)
    tree(w, rng, f"{path}\\bin", n(s, rng.randint(300, 1_500)), [("dll", 6), ("pdb", 2), ("exe", 1), ("json", 1)], 3, mean_days=20)
    tree(w, rng, f"{path}\\obj", n(s, rng.randint(300, 1_500)), [("dll", 2), ("cache", 3), ("json", 2), ("cs", 2)], 3, mean_days=20)
    git(w, rng, f"{path}\\.git", s, rng.randint(300, 2_000))


def user(w, rng, root, s):
    w.folder(f"{root}\\Users")
    u = f"{root}\\Users\\bench"
    w.folder(u)
    ad = f"{u}\\AppData"
    w.folder(ad)
    loc = f"{ad}\\Local"
    w.folder(loc)
    chrome = f"{loc}\\Google\\Chrome\\User Data\\Default"
    w.folder(f"{loc}\\Google")
    w.folder(f"{loc}\\Google\\Chrome")
    w.folder(f"{loc}\\Google\\Chrome\\User Data")
    w.folder(chrome)
    w.folder(f"{chrome}\\Cache")
    flat(w, rng, f"{chrome}\\Cache\\Cache_Data", n(s, 25_000), [("", 1)], 20, name=lambda i: f"f_{i:06x}")
    tree(w, rng, f"{chrome}\\Code Cache", n(s, 8_000), [("", 1)], 2, mean_days=20)
    tree(w, rng, f"{chrome}\\Service Worker\\CacheStorage", n(s, 6_000), [("", 1)], 3, mean_days=20)
    tree(w, rng, f"{loc}\\Microsoft\\Edge\\User Data\\Default\\Cache", n(s, 15_000), [("", 1)], 2, mean_days=20)
    npm = f"{loc}\\npm-cache\\_cacache"
    w.folder(f"{loc}\\npm-cache")
    w.folder(npm)
    for part, count in (("content-v2\\sha512", 90_000), ("index-v5", 90_000)):
        base = f"{npm}\\{part}"
        w.folder(f"{npm}\\{part.split(chr(92))[0]}")
        if "\\" in part:
            w.folder(base)
        buckets = {}
        for _ in range(n(s, count)):
            buckets.setdefault(f"{rng.getrandbits(8):02x}", {}).setdefault(f"{rng.getrandbits(8):02x}", []).append(f"{rng.getrandbits(240):060x}")
        for a in sorted(buckets):
            w.folder(f"{base}\\{a}")
            for b in sorted(buckets[a]):
                w.folder(f"{base}\\{a}\\{b}")
                for h in buckets[a][b]:
                    w.file(f"{base}\\{a}\\{b}\\{h}", size_of(rng, "js") * 3, 120)
    tree(w, rng, f"{loc}\\pip\\cache\\http-v2", n(s, 20_000), [("", 3), ("body", 2)], 5, (4, 16), 120)
    tree(w, rng, f"{loc}\\Temp", n(s, 8_000), [("tmp", 5), ("log", 3), ("etl", 1), ("txt", 1), ("dll", 1), ("exe", 1), ("zip", 1)], 3, mean_days=40)
    w.folder(f"{loc}\\Packages")
    for i in range(n(s, 120)):
        tree(w, rng, f"{loc}\\Packages\\Microsoft.{word(rng).capitalize()}_{rng.getrandbits(52):013x}", rng.randint(5, 80), [("dat", 3), ("db", 1), ("json", 2), ("png", 2), ("xml", 1)], 3)
    tree(w, rng, f"{loc}\\JetBrains\\IntelliJIdea2025.2\\caches", n(s, 15_000), [("dat", 3), ("len", 1), ("keystream", 1), ("values", 1)], 4)
    tree(w, rng, f"{loc}\\Microsoft\\WindowsApps", n(s, 400), [("exe", 1)], 2)
    ro = f"{ad}\\Roaming"
    w.folder(ro)
    tree(w, rng, f"{ro}\\Code\\User\\workspaceStorage", n(s, 2_000), [("json", 2), ("vscdb", 1), ("db", 1)], 2)
    tree(w, rng, f"{ro}\\discord\\Cache\\Cache_Data", n(s, 5_000), [("", 1)], 1)
    tree(w, rng, f"{ro}\\Microsoft", n(s, 3_000), [("lnk", 5), ("dat", 1), ("xml", 1)], 4)
    cargo = f"{u}\\.cargo\\registry"
    w.folder(f"{u}\\.cargo")
    w.folder(cargo)
    w.folder(f"{cargo}\\src")
    w.folder(f"{cargo}\\src\\index.crates.io-1949cf8c6b5b557f")
    w.folder(f"{cargo}\\cache")
    w.folder(f"{cargo}\\cache\\index.crates.io-1949cf8c6b5b557f")
    for i in range(n(s, 400)):
        c = f"{word(rng)}-{rng.randint(0, 3)}.{rng.randint(0, 40)}.{rng.randint(0, 20)}"
        tree(w, rng, f"{cargo}\\src\\index.crates.io-1949cf8c6b5b557f\\{c}", rng.randint(10, 150), [("rs", 10), ("toml", 1), ("md", 1)], 3, mean_days=150)
        w.file(f"{cargo}\\cache\\index.crates.io-1949cf8c6b5b557f\\{c}.crate", size_of(rng, "crate"), 150)
    tree(w, rng, f"{u}\\.rustup\\toolchains\\stable-x86_64-pc-windows-msvc", n(s, 30_000), [("html", 20), ("js", 1), ("rlib", 3), ("dll", 1), ("exe", 1), ("css", 1)], 5, mean_days=60)
    w.folder(f"{u}\\.nuget")
    tree(w, rng, f"{u}\\.nuget\\packages", n(s, 9_000), [("dll", 4), ("nupkg", 1), ("xml", 3), ("json", 1)], 4)
    tree(w, rng, f"{u}\\Documents", n(s, 4_000), [("docx", 4), ("pdf", 5), ("xlsx", 3), ("txt", 2), ("pptx", 1)], 5, mean_days=400)
    pics = f"{u}\\Pictures"
    w.folder(pics)
    for y in range(2016, 2026):
        w.folder(f"{pics}\\{y}")
        for m in range(1, 13):
            flat(w, rng, f"{pics}\\{y}\\{y}-{m:02d}", n(s, rng.randint(50, 360)), [("jpg", 7), ("heic", 3), ("png", 1)], 700,
                 name=lambda i: f"IMG_{rng.randint(1000, 9999)}{i}")
    tree(w, rng, f"{u}\\Videos", n(s, 300), [("mp4", 4), ("mov", 1)], 2, mean_days=500)
    mus = f"{u}\\Music"
    w.folder(mus)
    for a in range(n(s, 250)):
        artist = f"{mus}\\{word(rng).capitalize()} {word(rng).capitalize()}"
        w.folder(artist)
        for b in range(rng.randint(1, 3)):
            flat(w, rng, f"{artist}\\{word(rng, 2, 5).capitalize()}", rng.randint(6, 16), [("mp3", 3), ("flac", 1)], 1500,
                 name=lambda i: f"{i + 1:02d} {word(rng, 2, 4).capitalize()}")
    flat(w, rng, f"{u}\\Downloads", n(s, 1_500), [("zip", 4), ("exe", 3), ("msi", 2), ("pdf", 4), ("iso", 0.15), ("png", 2), ("jpg", 2)], 200)
    tree(w, rng, f"{u}\\OneDrive", n(s, 15_000), [("docx", 4), ("pdf", 4), ("jpg", 8), ("xlsx", 2), ("txt", 2), ("mp4", 1)], 5, mean_days=500)
    src = f"{u}\\source"
    w.folder(src)
    w.folder(f"{src}\\repos")
    used = set()
    for kind, count in ((js_project, 25), (rust_project, 10), (python_project, 6), (dotnet_project, 4)):
        for _ in range(count):
            name = word(rng, 2, 3)
            while name in used:
                name = word(rng, 2, 3)
            used.add(name)
            kind(w, rng, f"{src}\\repos\\{name}", s)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", default=os.path.join("target", "bench", "large.csv"))
    ap.add_argument("--scale", type=float, default=1.0)
    ap.add_argument("--root", default="Q:")
    ap.add_argument("--seed", type=int, default=20261123)
    a = ap.parse_args()
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    rng = random.Random(a.seed)
    root = a.root.rstrip("\\")
    tmp = a.out + ".part"
    with open(tmp, "w", encoding="utf-8", newline="", buffering=1 << 22) as out:
        out.write("﻿Generated by tools/bench-data.py (synthetic)\r\n")
        out.write("File Name,Size,Allocated,Modified,Attributes,Files,Folders\r\n")
        w = Writer(out, rng)
        w.folder(root)
        for part in (windows, program_files, program_data, games, user):
            part(w, rng, root, a.scale)
    os.replace(tmp, a.out)
    print(f"{a.out}: {w.files:,} files, {w.folders:,} folders, {w.bytes / 2**30:,.1f} GiB")


if __name__ == "__main__":
    main()
