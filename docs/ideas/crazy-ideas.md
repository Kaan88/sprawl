# Sprawl: crazy ideas backlog

Baseline already in progress: acrylic window, D3D11 instanced lit-box treemap in a SwapChainPanel (cushion shading, hover lift + ripple, DirectWrite labels, click-to-zoom morph), scan HUD (MFT block map, counters, sparkline, file-type race), virtualized sortable tables, MFT scanning.

Scoring: Effort S (days or less), M (about a week), L (2-3 weeks), XL (month+). Wow 1-5.
Shorthand: D3D11 = `windows` crate `Win32::Graphics::Direct3D11`, D2D = Direct2D, DWrite = DirectWrite, WinUI = Windows App SDK 1.8 XAML, SCP = `SwapChainPanel` (via `ISwapChainPanelNative::SetSwapChain`).

---

## A. Rendering techniques (make the same treemap gorgeous)

### A1. SDF / MSDF text for treemap labels
- **Sees:** Folder and file names stay razor sharp at any zoom, scale smoothly during the click-to-zoom morph, tilt in perspective with the camera, and get a soft glow or outline when the block is hovered. No shimmering or re-rasterising during animation.
- **Useful:** Labels are the main readability problem on dense treemaps, and per-frame DirectWrite re-layout is the typical perf cliff. Atlas text draws 10k labels in one instanced call and fades them by block pixel size (LOD).
- **Build:** Generate an MSDF atlas for Segoe UI Variable once (msdfgen offline, or at runtime: DWrite glyph outlines -> distance transform in a compute shader). Instanced glyph quads in D3D11, median-of-three + `fwidth` AA in the pixel shader. Shape text with `IDWriteTextAnalyzer::GetGlyphs/GetGlyphPlacements`, cache per name.
- **Effort:** L. **Wow:** 3 (invisible when right, but everything 3D depends on it)

### A2. HDR bloom and tonemapping on hot blocks
- **Sees:** The biggest or hovered blocks emit a soft halo, specular highlights bloom on glossy edges, and on HDR monitors highlights exceed SDR white. Selected file types pulse with a gentle glow.
- **Useful:** Bloom is a free attention channel: glow = what matters (biggest, selected, just-changed).
- **Build:** Render to `DXGI_FORMAT_R16G16B16A16_FLOAT`; bright-pass -> dual-Kawase down/up chain (pixel or compute shaders) -> composite with ACES/AgX tonemap. Swap chain in FP16 with `DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709` when `IDXGIOutput6::GetDesc1` reports HDR10.
- **Effort:** M. **Wow:** 4

### A3. SSAO / GTAO contact shadows between blocks
- **Sees:** Gutters between blocks darken softly, tall blocks cast ambient occlusion on neighbours, and the map gets real depth like a macro photo of a circuit board.
- **Useful:** Separates small nested blocks without borders (less visual noise than outlines).
- **Build:** Depth + normal G-buffer (MRT), GTAO compute pass at half-res with bilateral blur, multiplied into lighting. Cheaper alternative: analytic per-instance AO from neighbour heights in a structured buffer, since the geometry is boxes.
- **Effort:** M. **Wow:** 3

### A4. Physically based material modes: glass, brushed metal, neon, holo foil
- **Sees:** A material switcher: Cushion (classic), Frosted glass (blocks refract and blur what is behind them), Brushed metal, Neon wireframe, Holographic foil (iridescent by view angle). Switching morphs materials over 400 ms.
- **Useful:** Mostly delight, but neon wireframe doubles as a high-contrast accessibility mode, and glass makes hierarchy legible (nested folders become translucent shells around children).
- **Build:** Cook-Torrance GGX in the pixel shader, procedural prefiltered cubemap for IBL (compute pass), thin-film iridescence as a cosine palette on `N.V`. Glass: copy the opaque pass to a texture, sample with normal-based offset, mip chain for roughness blur.
- **Effort:** L. **Wow:** 4

### A5. Depth of field and rack-focus on zoom
- **Sees:** When you click a folder and the camera dives in, everything off the focus plane blurs into bokeh, then snaps sharp as the zoom settles, like a cinema rack focus.
- **Useful:** Directs the eye to the zoom target and masks layout re-flow during the morph.
- **Build:** Circle of confusion from linear depth, gather-based DoF compute shader at half-res (16-32 Poisson taps), focus distance driven by the same spring as the camera.
- **Effort:** M. **Wow:** 3

### A6. Glossy mirror floor
- **Sees:** The 3D treemap sits on a dark glossy floor and reflects itself, the reflection fading with distance and blurred by roughness. Hovered blocks reflect their glow.
- **Useful:** Mostly aesthetic, but grounds the 3D look so blocks do not float.
- **Build:** Planar reflection: render the instanced scene again with a mirrored view matrix and clip plane (exact for a flat floor), blur by mip level, Fresnel blend.
- **Effort:** S. **Wow:** 3

### A7. Volumetric light shafts and fog
- **Sees:** A soft god-ray beam sweeps from a light above, picking out dust around the tallest blocks, and fog thickens toward the horizon in the city view.
- **Useful:** Depth cue for the 3D camera modes; helps judge distance in fly-through.
- **Build:** Froxel volume ray-march in compute (shadow map from a directional light, 64 steps, temporal jitter), composited additively before tonemap.
- **Effort:** L. **Wow:** 3

### A8. GPU particles for scan and delete events
- **Sees:** During the scan, glowing motes stream from the MFT block map into the treemap cells where files land. Deleting a folder shatters its block into cubes that fall and fade, and the freed space dissolves into mist.
- **Useful:** Deletion feedback ("did that work?") and a legible metaphor for bytes moving.
- **Build:** Compute-shader particle sim with `RWStructuredBuffer` append/consume buffers and `DrawInstancedIndirect`, spawned from the dying block's instance rect. 1M particles is easy on a 4070 Ti Super.
- **Effort:** M. **Wow:** 4

### A9. Movable sun with soft shadows and time-of-day
- **Sees:** A draggable "sun" handle casts long soft shadows across the map; tall blocks shadow smaller ones. A time-of-day slider changes light colour (golden-hour treemap).
- **Useful:** Shadow length is a second size encoding in city view; the slider is a screenshot generator.
- **Build:** Single 4096 shadow map with PCSS, or, since the scene is a heightfield, a compute horizon-shadow pass in screen space.
- **Effort:** M. **Wow:** 3

### A10. Backdrop colour sync (ambient bias lighting)
- **Sees:** The acrylic window tint slowly shifts to the treemap's dominant colours, like TV bias lighting, so the chrome never clashes with the data.
- **Useful:** Cohesion and polish. On HDR displays also use wide-gamut P3 colours.
- **Build:** Mip-chain the treemap to 1x1 on the GPU and read it back one frame late via a staging texture; animate `DesktopAcrylicController` tint colour / `TintOpacity` (`Microsoft.UI.Composition.SystemBackdrops`).
- **Effort:** S. **Wow:** 3

---

## B. Alternative 3D views (same data, new shapes)

### B1. City skyline view (extruded treemap)
- **Sees:** The flat treemap rises into a skyline: block height = file count, depth or age (selectable), footprint = size. Orbit, tilt and zoom with inertia; lit windows on towers make big folders look like skyscrapers at night.
- **Useful:** A second metric: file count vs size shows "many tiny files" folders (node_modules) as dense needles.
- **Build:** Instanced boxes already exist: add a per-instance height from a metric buffer, an orbit camera with critically-damped springs, and a procedural window shader from the instance hash. Height morph is a lerp in the vertex shader driven by a uniform.
- **Effort:** S (given the current engine). **Wow:** 5

### B2. Extruded sunburst ("vinyl record") view
- **Sees:** Concentric rings, one per depth level, arcs sized by bytes, extruded and glossy like a record. Click a segment and the disc zooms into it; during scans it slowly spins like a hard drive platter.
- **Useful:** Sunburst shows hierarchy depth better than a treemap, and people love the DaisyDisk look.
- **Build:** Instanced annular sectors generated in the vertex shader from (r0, r1, a0, a1) per instance; same picking path as the treemap. Layout in `sprawl-core` (new `sunburst.rs`).
- **Effort:** M. **Wow:** 4

### B3. Galaxy / solar-system view
- **Sees:** The drive is a sun, top folders are planets sized by bytes, sub-folders orbit as moons, files are a dust cloud of stars. Orbit speed follows recent modification, so active folders swirl and cold archives barely move. Fly the camera into a planet to see its moons.
- **Useful:** Good at showing what is "hot" (speed and brightness encode activity), and the most shareable screenshot.
- **Build:** Deterministic layout (orbit radius from depth, angle from path hash), 10M-point star field via instanced billboards + additive blend, bloom (A2), distance LOD that aggregates points. Free-fly camera with WASD or `Windows.Gaming.Input.Gamepad`.
- **Effort:** L. **Wow:** 5

### B4. Terrain heightfield / mountain range
- **Sees:** The drive becomes a mountain range: each folder a mountain whose height is its size, snow-capped for huge ones, valleys for empty space. A glowing contour toggle turns it into a topographic chart.
- **Useful:** A smooth surface shows distribution at a glance (one Everest vs many hills), and contour lines give quantitative reading.
- **Build:** Splat folder rects into a heightmap texture via compute (gaussian per folder), displace a tessellated grid (`HS/DS` shaders), triplanar rock/snow shading, marching-squares contours in the pixel shader using `fwidth`.
- **Effort:** L. **Wow:** 4

### B5. Fly-through drone mode
- **Sees:** Drop into the city view as a drone: WASD + mouse look (or gamepad) through canyons between folders, with the path breadcrumb floating above and labels billboarding to face you. Press Space to land on a block and open its details.
- **Useful:** Exploring dense trees is more fun and less lossy than repeated click-to-zoom; also a demo-reel mode.
- **Build:** Raw Input (`RegisterRawInputDevices`) or XAML key/pointer events, `Windows.Gaming.Input`, collision against an instance AABB grid, frustum + HiZ occlusion culling in compute.
- **Effort:** M. **Wow:** 4

### B6. Circuit-board arc view for duplicates and links
- **Sees:** Blocks fade back and glowing arcs fly between files sharing content (duplicates) or hard links/junctions, like data-flow lines on a PCB. Hover a file and its twins light up across the map with traced curves.
- **Useful:** Duplicates are spatially scattered; arcs show where redundancy lives (see D2).
- **Build:** Instanced bezier ribbons (vertex shader expands a quad strip per arc), additive blend + bloom, arcs animated with a dash-offset uniform.
- **Effort:** M. **Wow:** 4

---

## C. Scan-time spectacle (live visualisations)

### C1. Live treemap that grows as the scan runs
- **Sees:** Blocks pop into existence with a springy overshoot as folders are discovered; the layout continuously re-squarifies with cells sliding to new positions rather than jumping. Big finds flash briefly.
- **Useful:** Immediate feedback and early insight: you often spot the culprit folder at 20% and can cancel.
- **Build:** Throttled snapshots from the scanner (lock-free double buffer of folder totals); layout at about 10 Hz in `sprawl-core`; each instance stores (rect_prev, rect_next, t0) and the vertex shader springs between them.
- **Effort:** M. **Wow:** 5

### C2. MFT radar with a physical-disk sweep
- **Sees:** The MFT block map becomes a circular platter: a sweep line reads records clockwise, lit cells are record clusters coloured by file type. Fragmented MFT regions show as shattered arcs.
- **Useful:** Accurate "where in the MFT am I" progress, and it exposes sparse or fragmented MFTs (real diagnostic value).
- **Build:** Scanner reports record index ranges per chunk (`FSCTL_GET_NTFS_VOLUME_DATA`, `FSCTL_GET_RETRIEVAL_POINTERS` for `$MFT` extents). Polar plot in D2D or a small compute shader writing a 512x512 R8 texture.
- **Effort:** M. **Wow:** 4

### C3. Throughput tachometer and I/O heat strip
- **Sees:** An analogue gauge with a needle that bounces on MB/s and records/s, plus a scrolling heat strip of queue depth and latency per second, so you see the drive's pulse.
- **Useful:** Tells you whether you are I/O-bound (HDD) or CPU-bound (parsing), which guides tuning.
- **Build:** PDH counters (`PdhAddCounter` on `\PhysicalDisk(*)\Avg. Disk Queue Length`) plus own counters; draw with D2D in a small SCP or XAML `Canvas`.
- **Effort:** S. **Wow:** 3

### C4. File-type race with overtake animations
- **Sees:** The race bars physically swap places with a swoosh, shell icons ride the bar tips, and a podium flash plays when the final tally lands.
- **Useful:** Shows what the drive is made of; animated swaps are bar-chart-race screenshot gold.
- **Build:** WinUI Composition implicit animations (`ImplicitAnimationCollection`, `Offset`) on a virtualised bar list; icons from `SHGetFileInfo` or `IShellItemImageFactory`.
- **Effort:** S. **Wow:** 3

### C5. "Treasure find" callouts during scan
- **Sees:** Callouts pop up on the treemap: "Found 41 GB hiberfil.sys", "node_modules x 212 folders = 18 GB", "Largest file so far...". They attach to their block with a leader line and fade after 4 seconds.
- **Useful:** Surfaces insights while you wait, so scan time is productive.
- **Build:** Scanner emits notable events (new max file, folder crosses 1 GB, known-pattern match) into a channel; UI projects the block centre to screen and draws callouts with `ExpressionAnimation` tracking.
- **Effort:** S. **Wow:** 3

---

## D. Understanding the data (smart and useful)

### D1. Time-lapse replay: watch the disk fill up
- **Sees:** A scrubber under the treemap; press play and blocks appear in the order files were created (or modified), growing year by year (2014 ... 2026) with a date ticker and cumulative area chart. A 90 GB game install in March 2023 causes a visible lurch.
- **Useful:** Answers "when did this folder balloon and why", and finds forgotten ancient data. Colouring by age also reveals stale gigabytes.
- **Build:** Keep created/modified per record from the MFT (`$STANDARD_INFORMATION` times); sort ids by time into a GPU buffer; the instance shader tests `fileTime <= currentTime` and scales height/area (layout fixed at "now", so cells grow in place: cheap and stable). Date axis in D2D.
- **Effort:** M. **Wow:** 5

### D2. Duplicate finder as constellations
- **Sees:** After a hash pass, duplicate groups are tagged with the same pulsing colour; hover one and all copies glow and connect with arcs (B6). A "reclaimable" counter ticks up as you mark copies, and marked blocks go ghostly, previewing the freed space.
- **Useful:** Duplicates are the classic win; the map shows which folders are the redundant ones. Preview-before-delete lowers fear of deleting.
- **Build:** Staged hashing: size bucket -> first 4 KB (xxh3) -> full file (BLAKE3 crate, `FILE_FLAG_SEQUENTIAL_SCAN`, overlapped I/O, rayon); group ids to a GPU buffer. Offer "dedupe instead of delete" via `CreateHardLinkW` or `FSCTL_DUPLICATE_EXTENTS_TO_FILE` (ReFS / Dev Drive).
- **Effort:** L. **Wow:** 4

### D3. Scan diff animation: what changed since last time
- **Sees:** Load a previous snapshot (saved automatically) and the map morphs between then and now: growth in glowing red-orange, shrinkage in cool blue, new files as sparks, vanished files as ghost outlines. A top-10 "movers" list updates with the animation.
- **Useful:** "Where did my 40 GB go this week?" is the number-one real question.
- **Build:** Compact columnar snapshot (path hash, parent, size, mtime) via `bincode`/`rkyv` + zstd. Join by path hash in `sprawl-core`; per-instance (size_old, size_new) lerped in the shader; diverging colour ramp.
- **Effort:** L. **Wow:** 5

### D4. Live file-system watcher: the map breathes
- **Sees:** After the scan the treemap stays alive: when a download writes to a folder its block grows with a green pulse; deletes shrink with a blue ripple. Browser caches and logs visibly twitch. A "Live" badge shows events per second.
- **Useful:** Finds what is eating disk right now (runaway logs, a build filling `target\`), and proves the data stays fresh without rescanning.
- **Build:** USN journal: `FSCTL_QUERY_USN_JOURNAL` / `FSCTL_READ_USN_JOURNAL` (`READ_USN_JOURNAL_DATA_V1`) on a background thread; resolve FRNs via the in-memory MFT index or `OpenFileById`. Fall back to `ReadDirectoryChangesExW` for non-NTFS. Apply size deltas up the ancestor chain; partial GPU instance updates (`UpdateSubresource` on dirty ranges).
- **Effort:** L. **Wow:** 5

### D5. Lenses: heat by age, type, owner, cloud state
- **Sees:** One-key lenses recolour the map: last accessed (red = hot, steel blue = untouched 3 years), file type, owner, compression ratio, cloud status (OneDrive placeholder vs local). A legend fades in at the corner.
- **Useful:** Fast triage of what can be archived, offloaded or compressed.
- **Build:** Per-instance attribute buffer plus colour-ramp texture lookup in the pixel shader; lens switch crossfades two attribute buffers. Cloud state from `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` / `FILE_ATTRIBUTE_PINNED`; compression from `GetCompressedFileSizeW`.
- **Effort:** M. **Wow:** 3

### D6. "Explain this block" smart categoriser
- **Sees:** Hovering a mystery folder (`C:\ProgramData\Package Cache`) shows a card: "Installer cache, safe to prune with Disk Cleanup, 3.2 GB, 94% confidence", with a "Show me how" button. Clusters get auto-names such as "Games", "Dev toolchains", "Photos 2019-2022".
- **Useful:** Fear of deleting the wrong thing is the real barrier; explanations turn sizes into decisions.
- **Build:** Tier 1: shipped, updatable JSON rule database (path globs + safety class). Tier 2: extension-mix heuristics and magic bytes (`infer` crate). Tier 3 (optional): a local model via Windows AI APIs (Phi Silica) or ONNX Runtime + DirectML, fed only paths and extension histograms, never contents.
- **Effort:** L. **Wow:** 4

### D7. Cleanup what-if basket with ghost preview
- **Sees:** Tick several folders into a basket; the treemap ghosts them and a big free-space bar animates to the predicted result ("+86 GB free, 71% -> 48%"). Commit sends them to the Recycle Bin in one operation with an undo toast.
- **Useful:** Turns browsing into decisions with a clear payoff, and prevents regret.
- **Build:** Basket in `sprawl-core`; ghost = per-instance flag + alpha. Delete via `IFileOperation` (`FOF_ALLOWUNDO`) for Recycle Bin and native progress UI; undo by restoring from the bin.
- **Effort:** M. **Wow:** 3

### D8. Compare two drives or machines (split scene)
- **Sees:** Two treemaps in one scene, matching folders linked by glowing threads; a slider wipes between "C:" and a backup "E:" and highlights what is missing from the backup in red.
- **Useful:** Backup verification and migration checks.
- **Build:** Two snapshots (D3 format), path join, a vertical split via a pixel-shader mask uniform, drag handle as a XAML `Thumb` overlay.
- **Effort:** M. **Wow:** 3

---

## E. Interaction and input

### E1. Spring physics and inertia everywhere
- **Sees:** Everything has mass: orbit with inertia, exponential zoom toward the cursor, hover lift with spring overshoot, panels settling with a tiny bounce. Nothing moves linearly.
- **Useful:** Subtle, but it is what makes the app feel premium and reduces disorientation after zoom.
- **Build:** One `Spring<T>` (critically damped, analytic) utility in Rust used by camera, hover and layout morphs; fixed sub-stepping decoupled from vsync via `QueryPerformanceCounter`; pacing with `IDXGISwapChain2::GetFrameLatencyWaitableObject`.
- **Effort:** S. **Wow:** 4

### E2. Touch and pen gestures
- **Sees:** Pinch to zoom, two-finger twist to orbit, flick to throw the camera, long-press for a radial menu around the finger. With a Surface Pen, hover previews and tap-to-select.
- **Useful:** Tablet/2-in-1 usability and demo appeal.
- **Build:** WinUI `ManipulationMode` (`Scale|Rotate|TranslateInertia`) + `ManipulationDelta` on the SCP, or `GestureRecognizer`; `PointerDeviceType.Pen` hover events; radial menu as a XAML `Popup` with composition animations.
- **Effort:** M. **Wow:** 3

### E3. GPU id-buffer picking, radial context menu, lasso select
- **Sees:** Right-click a block and a ring of icon actions fans out (Open in Explorer, Delete, Copy path, Properties, Exclude, Add to basket) with magnetic hover. Shift-drag lassos many blocks in the 3D scene with a glowing rectangle.
- **Useful:** Faster than list menus, and selection works across the whole map. O(1) hover even with millions of instances.
- **Build:** Render instance ids to an `R32_UINT` target and read back the pixel/rectangle via a staging texture (replaces CPU ray picking). Radial menu: `Popup` with `Path` wedges and `ScalarKeyFrameAnimation`.
- **Effort:** M. **Wow:** 3

### E4. Command palette with live filter (Ctrl+K)
- **Sees:** A frosted search box; typing `*.iso >1gb` filters the map live: non-matches sink and fade, matches rise and glow, with count and total size updating each keystroke. Phrases like "videos older than 2 years" work.
- **Useful:** Search, filter and a visual answer in one place; the fastest way to find space hogs.
- **Build:** Query parser in `sprawl-core` (nom) over columnar arrays with SIMD size/time compares; results as a bitset uploaded as a `StructuredBuffer<uint>` mask; palette overlay with an acrylic brush. Optional natural-language parse via the local model (D6).
- **Effort:** M. **Wow:** 4

### E5. Magnifier lens (hold Alt)
- **Sees:** A circular lens follows the cursor: inside it the treemap is rendered at 8x detail with all sub-files and labels, while outside dims slightly. The lens edge refracts like glass.
- **Useful:** Reading tiny blocks without losing context or committing to a zoom.
- **Build:** Render a second narrow-FOV view of the instance buffer into a small target (or radial vertex distortion), composite with a circular SDF mask and refraction normal.
- **Effort:** M. **Wow:** 4

---

## F. Audio (optional, off by default)

### F1. Sonification of scan and hover
- **Sees:** (Hears.) A soft Geiger-style tick per thousand files with pitch rising with throughput, and a chord resolving on completion. Hovering a block plays a note whose pitch maps to size (big = low, bassy), so you can hear where the weight is while sweeping the mouse.
- **Useful:** Ambient progress feedback and a secondary channel for accessibility.
- **Build:** XAudio2 with a small procedural synth (sine/FM, short envelopes), pan by block x-position, pooled source voices for low latency. Global mute setting.
- **Effort:** S. **Wow:** 3

---

## G. Windows integration

### G1. Taskbar progress, overlay and Jump List
- **Sees:** The taskbar icon fills green during the scan (paused yellow, error red) with a percent badge, and the Jump List lists recent scans ("C:\ 412 GB") plus tasks "Scan C:", "Scan all drives".
- **Useful:** Alt-tab away from a long scan and still know its status; recents cut clicks.
- **Build:** `ITaskbarList3::SetProgressValue/SetProgressState/SetOverlayIcon`; Jump List via `ICustomDestinationList` + `IShellLinkW`; `SetCurrentProcessExplicitAppUserModelID`.
- **Effort:** S. **Wow:** 2 (expected polish)

### G2. Mica Alt title bar and correct Snap Layouts
- **Sees:** The title bar merges with a Mica Alt strip, sidebars use in-app acrylic, the treemap sits on a deeper dark pane, and hovering the maximise button shows the Windows 11 snap flyout.
- **Useful:** Native feel; custom WinUI title bars often break snap layouts.
- **Build:** `MicaBackdrop { Kind = MicaKind.BaseAlt }` or `DesktopAcrylicBackdrop`; `AppWindow.TitleBar.ExtendsContentIntoTitleBar`, `SetTitleBar`, and `InputNonClientPointerSource.SetRegionRects` for caption regions.
- **Effort:** S. **Wow:** 3

### G3. Windows 11 widget with live drive rings
- **Sees:** A widget-board tile with a ring per drive (used/free, trend "+2.1 GB today") and a "Top 3 space hogs" button that deep-links into Sprawl at that folder.
- **Useful:** Passive awareness that keeps users returning.
- **Build:** Windows App SDK widget provider (COM server implementing `IWidgetProvider`, Adaptive Cards JSON), data from the cached snapshot and `GetDiskFreeSpaceExW`. Requires MSIX packaging.
- **Effort:** L. **Wow:** 3

### G4. Explorer integration: "Analyse with Sprawl"
- **Sees:** Right-click any folder in Explorer: "Sprawl this folder" opens straight into the zoomed treemap. Optionally a tiny treemap thumbnail appears in the Details pane.
- **Useful:** Entry point from where users already are.
- **Build:** Windows 11 `IExplorerCommand` COM verb via MSIX sparse package (`com.microsoft.windows.fileExplorerContextMenus`); `IThumbnailProvider` serving a cached PNG. CLI already accepts a folder argument.
- **Effort:** M. **Wow:** 3

### G5. Space-hog specialists: WSL, Docker, VHDX, WinSxS, Dev Drive
- **Sees:** Drive picker shows Dev Drive / BitLocker / cloud badges. A WSL VHDX gets a card: "This virtual disk is 80 GB, compact it". WinSxS and Windows.old get explainers with one-click handoff to Storage Sense or DISM.
- **Useful:** These are the real sources of "mystery" usage and each has a dedicated fix.
- **Build:** `GetVolumeInformationW`, ReFS volume ioctls; VHDX detection by extension + header; actions launching `wsl --manage <distro> --set-sparse true`, `Optimize-VHD`, `DISM /Online /Cleanup-Image /StartComponentCleanup`.
- **Effort:** M. **Wow:** 3

### G6. True accounting: hard links, sparse, compressed, placeholders
- **Sees:** A toggle between apparent size, allocated size and unique on-disk size. The map visibly shrinks when WinSxS hard links are de-duplicated, and OneDrive placeholders become hollow outlines.
- **Useful:** Correctness. Analysers mislead on WinSxS and cloud files, and trust is a feature.
- **Build:** MFT `$DATA` run lists (allocated vs real, sparse runs), hard-link count from `$FILE_NAME` attributes, track seen FRNs so each is counted once.
- **Effort:** M. **Wow:** 2 (huge for credibility)

---

## H. Sharing and showmanship

### H1. Share card and cinematic export
- **Sees:** Press Ctrl+Shift+S: the camera does a 3-second orbital move, the app composes a 4K poster ("C: 1.8 TB, 62% full, biggest: ...") with the glowing treemap and type breakdown, and saves or copies PNG/MP4/GIF. A "Wrapped"-style year summary is one click away.
- **Useful:** Sharing is the growth engine; also handy for IT reports.
- **Build:** Offscreen 3840x2160 MSAA target, staging readback, PNG via WIC (`IWICBitmapEncoder`); video via Media Foundation Sink Writer (`MFCreateSinkWriterFromURL`, H.264/HEVC with `IMFDXGIDeviceManager` for zero-copy); GIF via the `gif` crate.
- **Effort:** M. **Wow:** 5

### H2. Screensaver / ambient presentation mode
- **Sees:** A fullscreen mode (or `.scr`) that slowly flies over your own disk as a glowing galaxy or city, with labels fading in and out. Also usable as an IT wall-display kiosk.
- **Useful:** Marketing and delight; kiosk dashboard.
- **Build:** Same renderer in a borderless fullscreen window (flip-model `IDXGISwapChain1`), autopilot spline camera; `.scr` is a renamed exe handling `/s /c /p`.
- **Effort:** S (given B1/B3). **Wow:** 4

### H3. Terminal treemap and headless report
- **Sees:** `sprawl --report C:\` prints a coloured block treemap in Windows Terminal (24-bit ANSI) plus a JSON of top folders for scripts, using the same layout engine.
- **Useful:** Servers, SSH, CI and bug reports; widens the audience.
- **Build:** `sprawl-core` squarify onto a character grid using half-block glyphs and truecolor escapes (`crossterm`); no UI dependencies.
- **Effort:** S. **Wow:** 2

---

# Top 8 to do next (ranked)

1. **C1 Live-growing treemap during scan (wow 5, M).** The most visible moment of the app, seen on every scan. It forces the instance buffer to carry prev/next rects and springs, which D1, D3 and D4 reuse.
2. **E1 Spring physics and inertia framework (wow 4, S).** A cheap, invisible multiplier. Do it before more animations are written, or each gets hand-tuned.
3. **B1 City skyline view (wow 5, S).** The instanced-box renderer already has what is needed: a height channel, an orbit camera, a window shader. Biggest screenshot payoff per line, and it adds a real second metric (file count).
4. **D3 Scan diff animation (wow 5, L), starting with the snapshot file.** The most useful item: "where did my space go". The snapshot format also underpins D1 and D8, so begin by saving a snapshot on every scan now.
5. **D1 Time-lapse replay (wow 5, M).** Derived from MFT timestamps already being read, no extra I/O, layout fixed so it is cheap. Shareable and analytically valuable.
6. **A2 HDR bloom + tonemapping with A1 SDF text (wow 4, M and L).** Bloom gives the "fancy" look instantly and an attention channel (glow = selected or changed). SDF text first, since label quality under tilt and zoom decides whether 3D feels pro or janky.
7. **D4 Live USN-journal watching (wow 5, L).** Makes the app feel alive and finds what is filling the disk right now. The journal reader also enables near-instant incremental rescans.
8. **H1 Share card / cinematic export (wow 5, M).** A growth lever: gives people a reason to post. Needs only an offscreen target and a camera spline, and doubles as a regression screenshot tool for `--screenshot`.

Honourable mentions: B3 galaxy view as a later hero feature, E4 command palette (best "useful" interaction), G1 and G2 as an afternoon of polish that makes the app feel native, and E3 id-buffer picking, which should be done early because hover, lasso and the radial menu all depend on it.

## Suggested sequencing

1. Foundations: springs (E1), GPU id-buffer picking (E3 core), per-instance attribute buffer (D5), SDF text (A1), snapshot format (D3 core).
2. Spectacle: live growth (C1), skyline (B1), bloom (A2), share card (H1).
3. Insight: replay (D1), diff (D3), watcher (D4), duplicates (D2), command palette (E4).
4. Hero views and polish: sunburst (B2), galaxy (B3), terrain (B4), audio (F1), widgets and shell integration (G3, G4).
