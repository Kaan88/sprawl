//! Direct3D 11 renderer for the treemap, drawn into a WinUI `SwapChainPanel`.
//!
//! Every tile is an instanced box (top + four sides) lit in the pixel shader with a cushion
//! normal, rendered with 4x MSAA into an offscreen target, resolved into the swap chain's back
//! buffer, and then folder labels are drawn on top with Direct2D/DirectWrite. The swap chain is
//! a composition swap chain with premultiplied alpha, so gaps between tiles show the window's
//! acrylic behind them.

use std::ffi::c_void;

use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::WaitForSingleObject;

const FORMAT: DXGI_FORMAT = DXGI_FORMAT_B8G8R8A8_UNORM;

const HLSL: &str = r#"
cbuffer Frame : register(b0) { float4x4 uVP; float4 uEye; float4 uLight; float4 uSize; float4 uRight; float4 uUp; };
struct VSIn { float3 pos : POSITION; float3 nor : NORMAL; float4 rect : RECT; float4 par : PARENT; float4 col : COLOR; float4 fx : FX; };
struct PSIn { float4 sv : SV_Position; float3 n : NORMAL; float2 uv : UV; float2 puv : PUV; float4 col : COLOR; float4 fx : FX; float3 w : WORLD; float2 wh : WH; float4 ex : EXTRA; };
PSIn vs(VSIn i) {
    PSIn o;
    float2 xy = i.rect.xy + i.pos.xy * i.rect.zw;
    float h = i.fx.x + i.fx.y;
    float3 w = float3(xy.x - uSize.x * 0.5, uSize.y * 0.5 - xy.y, i.pos.z * h);
    float flip = uSize.z > 0.5 ? -1.0 : 1.0; // reflection pass: mirrored below the floor
    w.z *= flip;
    o.sv = mul(uVP, float4(w, 1.0));
    o.n = float3(i.nor.x, -i.nor.y, i.nor.z * flip);
    o.uv = i.pos.xy;
    o.puv = (xy - i.par.xy) / max(i.par.zw, 1e-3);
    o.col = i.col; o.fx = i.fx; o.w = w; o.wh = i.rect.zw; o.ex = i.par;
    return o;
}
// Dither: the 8-bit target rounds every colour to 1/255, and the dark, slow gradients here (the
// glossy floor fading out, the deep-space sky, a cushion's shading) then show as steps. Noise of
// +-0.5/255 before that rounding (interleaved gradient noise, fixed to the pixel, so it doesn't
// crawl) turns the steps into a grain too fine to see, and leaves a flat colour as it was. Colour
// and alpha get different noise: over a light page the floor is mostly alpha, and the same noise
// on both would cancel there. Premultiplied: the colour stays within its alpha.
float ign(float2 p) { return frac(52.9829189 * frac(dot(p, float2(0.06711056, 0.00583715)))); }
float4 dither(float4 c, float2 p) {
    float n = (ign(p) - 0.5) * (0.999 / 255.0);
    float m = (ign(p + float2(37.0, 17.0)) - 0.5) * (0.999 / 255.0);
    float a = c.a < 0.999 ? saturate(c.a + m) : c.a;
    return float4(clamp(c.rgb + n, 0.0, a), a);
}
float4 ps_lit(PSIn i) {
    float3 base = i.col.rgb; float kind = i.col.a;
    if (kind > 1.5 && kind < 2.5) {
        // Glossy floor: dark glass that fades out away from the map, with a soft sheen.
        float2 d = max(abs(i.w.xy) - i.ex.xy, 0.0); // ex.xy: half size of what stands on the floor
        float a = 0.5 * exp(-length(d) / 90.0);
        float3 V = normalize(uEye.xyz - i.w); float3 H = normalize(normalize(uLight.xyz) + V);
        float sheen = pow(saturate(H.z), 90.0) * 0.35;
        float3 fc = float3(0.03, 0.035, 0.055) + sheen;
        return float4(fc * a, a);
    }
    bool mirrored = uSize.z > 0.5;
    bool top = mirrored ? i.n.z < -0.5 : i.n.z > 0.5;
    if (kind > 2.5) {
        // A building of the city view. ex = (seed, share of lit windows, warmth, accent strength).
        float3 L = normalize(uLight.xyz); float3 V = normalize(uEye.xyz - i.w); float3 H = normalize(L + V);
        float3 nn = normalize(i.n);
        float diff = max(dot(nn, L), 0.0);
        float3 c = base * (0.25 + 0.9 * diff) * (top ? 1.0 : 0.62) + pow(max(dot(nn, H), 0.0), 40.0) * (top ? 0.12 : 0.03);
        float3 acc = lerp(float3(0.35, 0.85, 1.0), float3(1.0, 0.62, 0.3), i.ex.z);
        float glow = i.fx.z;
        if (!top) {
            float face = abs(i.n.x) > 0.5 ? i.w.y : i.w.x;
            float2 cell = float2(face / 9.5, abs(i.w.z) / 10.5);
            float2 id = floor(cell); float2 f = frac(cell);
            float win = step(0.22, f.x) * step(f.x, 0.78) * step(0.25, f.y) * step(f.y, 0.75);
            float h1 = frac(sin(dot(id + i.ex.x * 57.0 + (abs(i.n.x) > 0.5 ? 3.1 : 0.0), float2(12.9898, 78.233))) * 43758.5453);
            float tw = floor(uEye.w * 0.18 + h1 * 13.0);
            float h2 = frac(sin(dot(id * 1.7 + tw * 0.37 + i.ex.x * 91.0, float2(39.346, 11.135))) * 43758.5453);
            float lit = step(h2, i.ex.y);
            float3 wc = lerp(float3(0.55, 0.82, 1.0), float3(1.0, 0.72, 0.36), i.ex.z) * (0.75 + 0.6 * h1);
            c *= 1.0 - 0.3 * win * (1.0 - lit);
            c += wc * win * lit * (1.1 + glow * 2.0);
        }
        float2 e = min(i.uv, 1.0 - i.uv) * i.wh;
        float edge = top ? 1.0 - smoothstep(0.0, 1.5, min(e.x, e.y)) : 0.0;
        c += acc * edge * i.ex.w * 0.9 + glow * acc * (top ? 0.8 : 0.3);
        float lum = dot(c, float3(0.3, 0.59, 0.11));
        c = lerp(c, lum.xxx * 0.32, i.fx.w);
        if (mirrored) {
            float a = 0.55 * exp(i.w.z / 40.0);
            return float4(c * a, a);
        }
        return float4(c, 1.0);
    }
    float3 n;
    if (top) {
        float c1 = kind > 0.5 ? 0.2 : 0.62 * uSize.w;
        float2 g = (1.0 - 2.0 * i.uv) * c1 + (1.0 - 2.0 * saturate(i.puv)) * 0.28 * uSize.w;
        n = normalize(float3(-g.x, g.y, 1.0));
    } else n = normalize(i.n);
    float3 L = normalize(uLight.xyz); float3 V = normalize(uEye.xyz - i.w); float3 H = normalize(L + V);
    float diff = max(dot(n, L), 0.0);
    float spec = pow(max(dot(n, H), 0.0), 48.0) * (kind > 0.5 ? 0.05 : 0.38);
    float3 c = base * (0.34 + 0.78 * diff) * (top ? 1.0 : 0.55) + spec;
    float2 e = min(i.uv, 1.0 - i.uv) * i.wh;
    float edge = top ? 1.0 - smoothstep(0.0, 1.4, min(e.x, e.y)) : 0.0;
    c *= 1.0 - edge * 0.45 * (1.0 - i.fx.z);
    float glow = i.fx.z;
    c += glow * (0.22 * base + float3(0.10, 0.13, 0.16)) + glow * edge * float3(0.75, 0.9, 1.0);
    float lum = dot(c, float3(0.3, 0.59, 0.11));
    c = lerp(c, lum.xxx * 0.32, i.fx.w);
    if (mirrored) {
        float a = 0.55 * exp(i.w.z / 24.0);
        return float4(c * a, a);
    }
    return float4(c, 1.0);
}
float4 ps(PSIn i) : SV_Target { return dither(ps_lit(i), i.sv.xy); }

struct SOut { float4 sv : SV_Position; float2 uv : UV; float4 col : COLOR; float4 fx : FX; };
SOut vs_star(VSIn i) {
    SOut o;
    float2 c = i.pos.xy * 2.0 - 1.0;
    float kind = i.fx.y;
    // the quad is bigger than the star so the halo fits; hovered stars swell a little
    float s = i.rect.w * (kind > 3.5 ? 1.0 : 4.0) * (1.0 + i.fx.x * 0.6);
    float3 w = i.rect.xyz + (uRight.xyz * c.x + uUp.xyz * c.y) * s;
    o.sv = mul(uVP, float4(w, 1.0));
    o.uv = c; o.col = i.col; o.fx = i.fx;
    return o;
}
float4 ps_star_lit(SOut i) {
    float d2 = dot(i.uv, i.uv);
    float kind = i.fx.y;
    if (kind > 3.5) {
        // deep-space backdrop, a little lighter toward the middle
        float3 sky = lerp(float3(0.035, 0.03, 0.075), float3(0.004, 0.005, 0.012), saturate(sqrt(d2) * 3.0));
        return float4(sky * 0.94, 0.94);
    }
    // outside the disc: nothing (additive, depth off: the same as not drawing it)
    if (d2 > 1.0) return 0.0.xxxx;
    float glow = i.fx.x;
    float3 col = i.col.rgb;
    float b = i.col.a;
    // a hot core (16x smaller than the quad) and a soft halo
    float core = exp(-d2 * 160.0);
    float halo = exp(-d2 * (kind > 2.5 ? 7.0 : 14.0)) * (kind > 1.5 && kind < 2.5 ? 0.6 : 0.32);
    float3 c = lerp(col, 1.0.xxx, 0.65) * core * 1.6 + col * halo;
    // big stars and the core get faint diffraction spikes
    if (kind > 2.5 || (kind < 0.5 && i.col.a > 0.8)) {
        float sp = exp(-abs(i.uv.x) * 70.0) + exp(-abs(i.uv.y) * 70.0);
        c += col * sp * (1.0 - sqrt(d2)) * 0.35;
    }
    // hovered or selected: a bright ring
    float r = sqrt(d2);
    if (kind > 2.5) glow = 0.0;
    c += glow * (exp(-pow((r - 0.42) * 14.0, 2.0)) * float3(0.8, 0.95, 1.0) + col * halo * 1.5);
    c *= b;
    float a = saturate(max(c.r, max(c.g, c.b)));
    return float4(c, a);
}
float4 ps_star(SOut i) : SV_Target { return dither(ps_star_lit(i), i.sv.xy); }
// The other views: plain coloured triangles and lines. uRight.w = 1: positions are DIPs of the
// panel (2D); 0: world positions under the camera (3D, already shaded).
struct MIn { float3 pos : POSITION; float4 col : COLOR; };
struct MOut { float4 sv : SV_Position; float4 col : COLOR; };
MOut vs_mesh(MIn i) {
    MOut o;
    if (uRight.w > 0.5) o.sv = float4(i.pos.x / uSize.x * 2.0 - 1.0, 1.0 - i.pos.y / uSize.y * 2.0, 0.5, 1.0);
    else o.sv = mul(uVP, float4(i.pos, 1.0));
    o.col = i.col;
    return o;
}
float4 ps_mesh(MOut i) : SV_Target { return dither(float4(i.col.rgb * i.col.a, i.col.a), i.sv.xy); }
"#;

/// One tile: rectangle and parent rectangle in layout units (DIPs), colour + kind (0 file, 1
/// folder plate), and effects (base height, lift, glow, dim).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Instance {
    pub rect: [f32; 4],
    pub parent: [f32; 4],
    pub color: [f32; 4],
    pub fx: [f32; 4],
}

/// A vertex of the other views' meshes: position (DIPs in 2D, world units in 3D) and straight
/// (not premultiplied) colour.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub col: [f32; 4],
}

pub struct Label {
    /// Top-left in DIPs.
    pub x: f32,
    pub y: f32,
    pub max_w: f32,
    pub text: String,
    pub strong: bool,
}

pub struct Frame<'a> {
    /// Column-major view-projection matrix.
    pub vp: [f32; 16],
    pub eye: [f32; 3],
    pub light: [f32; 3],
    /// Layout size in DIPs.
    pub size: [f32; 2],
    pub cushion: f32,
    /// Seconds since start (window flicker in the city view).
    pub time: f32,
    /// Draw the reflection and the glossy floor.
    pub mirror: bool,
    /// The floor slab, drawn between the reflection and the tiles.
    pub floor: Option<Instance>,
    pub instances: &'a [Instance],
    /// Galaxy view: billboards, rect = (x, y, z, radius), fx = (glow, kind).
    pub stars: &'a [Instance],
    /// Camera right and up vectors in world space, for the billboards.
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub labels: &'a [Label],
    /// The other views: triangles, then lines, and whether they are in panel DIPs.
    pub mesh: &'a [Vertex],
    pub lines: &'a [Vertex],
    pub mesh_2d: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Constants {
    vp: [f32; 16],
    eye: [f32; 4],
    light: [f32; 4],
    size: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}

#[windows_core::interface("63aad0b8-7c24-40ff-85a8-640d944cc325")]
unsafe trait ISwapChainPanelNative: windows_core::IUnknown {
    fn SetSwapChain(&self, swapchain: *mut c_void) -> windows_core::HRESULT;
}

pub struct Gpu {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    swap: IDXGISwapChain2,
    waitable: HANDLE,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    cube: ID3D11Buffer,
    cube_count: u32,
    inst: ID3D11Buffer,
    inst_cap: usize,
    cb: ID3D11Buffer,
    raster: ID3D11RasterizerState,
    depth_state: ID3D11DepthStencilState,
    blend: ID3D11BlendState,
    blend_add: ID3D11BlendState,
    depth_off: ID3D11DepthStencilState,
    vs_star: ID3D11VertexShader,
    ps_star: ID3D11PixelShader,
    cb_mirror: ID3D11Buffer,
    extra: ID3D11Buffer,
    vs_mesh: ID3D11VertexShader,
    ps_mesh: ID3D11PixelShader,
    layout_mesh: ID3D11InputLayout,
    cb_mesh: ID3D11Buffer,
    mesh_buf: ID3D11Buffer,
    mesh_cap: usize,
    msaa: Option<(ID3D11Texture2D, ID3D11RenderTargetView, ID3D11DepthStencilView)>,
    /// Antialiasing: samples per pixel (1 = off).
    samples: u32,
    d2d: ID2D1DeviceContext,
    dwrite_bold: IDWriteTextFormat,
    dwrite_normal: IDWriteTextFormat,
    white: ID2D1SolidColorBrush,
    soft: ID2D1SolidColorBrush,
    shadow: ID2D1SolidColorBrush,
    /// Swap chain size in physical pixels and the DIP-to-pixel scale.
    pub px: (u32, u32),
    pub scale: (f32, f32),
    /// Set to read the next frame back into `captured` (BGRA rows).
    pub want_capture: bool,
    pub captured: Option<(u32, u32, Vec<u8>)>,
}

fn hr<T>(r: windows::core::Result<T>, what: &str) -> Result<T, String> {
    r.map_err(|e| format!("{what}: {e}"))
}

fn compile(entry: &str, target: &str) -> Result<Vec<u8>, String> {
    let entry = std::ffi::CString::new(entry).unwrap();
    let target = std::ffi::CString::new(target).unwrap();
    let mut code: Option<ID3DBlob> = None;
    let mut err: Option<ID3DBlob> = None;
    let r = unsafe {
        D3DCompile(
            HLSL.as_ptr() as *const c_void,
            HLSL.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr() as *const u8),
            PCSTR(target.as_ptr() as *const u8),
            1 << 15, // D3DCOMPILE_OPTIMIZATION_LEVEL3
            0,
            &mut code,
            Some(&mut err),
        )
    };
    if let Err(e) = r {
        let msg = err
            .map(|b| unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize())).into_owned() })
            .unwrap_or_default();
        return Err(format!("shader {}: {e} {msg}", entry.to_string_lossy()));
    }
    let b = code.ok_or("no shader code")?;
    Ok(unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()).to_vec() })
}

fn cube() -> Vec<f32> {
    let mut v = Vec::new();
    let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], n: [f32; 3]| {
        for p in [a, b, c, a, c, d] {
            v.extend_from_slice(&p);
            v.extend_from_slice(&n);
        }
    };
    quad([0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.], [0., 0., 1.]);
    quad([0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.], [-1., 0., 0.]);
    quad([1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.], [1., 0., 0.]);
    quad([0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.], [0., -1., 0.]);
    quad([0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.], [0., 1., 0.]);
    v
}

fn buffer(device: &ID3D11Device, bytes: usize, bind: D3D11_BIND_FLAG, dynamic: bool, init: Option<&[u8]>) -> Result<ID3D11Buffer, String> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: bytes as u32,
        Usage: if dynamic { D3D11_USAGE_DYNAMIC } else { D3D11_USAGE_DEFAULT },
        BindFlags: bind.0 as u32,
        CPUAccessFlags: if dynamic { D3D11_CPU_ACCESS_WRITE.0 as u32 } else { 0 },
        ..Default::default()
    };
    let data = init.map(|d| D3D11_SUBRESOURCE_DATA { pSysMem: d.as_ptr() as *const c_void, ..Default::default() });
    let mut out = None;
    hr(unsafe { device.CreateBuffer(&desc, data.as_ref().map(|d| d as *const _), Some(&mut out)) }, "CreateBuffer")?;
    out.ok_or_else(|| "CreateBuffer returned nothing".into())
}

/// What [`Gpu::new`] needs first and the slowest of it: the device with its context, and the two
/// shaders compiled. About 60-80 ms, which [`prewarm`] moves off the window's thread so the
/// first frame of the map doesn't wait for it.
struct Warm {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    vsb: Vec<u8>,
    psb: Vec<u8>,
    /// The galaxy's stars and the 3D views' mesh.
    star: (Vec<u8>, Vec<u8>),
    mesh: (Vec<u8>, Vec<u8>),
}

// SAFETY: made on the warming thread, then handed over whole and used by the window's thread
// only; a D3D11 device is free-threaded, and its immediate context is used by one thread at a time.
unsafe impl Send for Warm {}

/// The warming thread, while its result hasn't been taken.
static WARM: std::sync::Mutex<Option<std::thread::JoinHandle<Result<Warm, String>>>> = std::sync::Mutex::new(None);

/// Start making the device and the shaders on their own thread, once, at launch: the map takes
/// them when it first draws ([`take_warm`]). Nothing is shown or changed by it.
pub fn prewarm() {
    if let Ok(mut w) = WARM.lock() {
        if w.is_none() {
            *w = std::thread::Builder::new().name("gpu warm".into()).spawn(make_warm).ok();
        }
    }
}

/// The warmed device and shaders, waiting for them if the thread is still at it; made here
/// instead when there was none (a second map after a lost device) or it failed.
fn take_warm() -> Result<Warm, String> {
    let started = WARM.lock().ok().and_then(|mut w| w.take());
    match started.map(|h| h.join()) {
        Some(Ok(Ok(w))) => Ok(w),
        _ => make_warm(),
    }
}

fn make_warm() -> Result<Warm, String> {
    let mut device = None;
    let mut ctx = None;
    hr(
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
        },
        "D3D11CreateDevice",
    )?;
    let device: ID3D11Device = device.ok_or("no device")?;
    let ctx: ID3D11DeviceContext = ctx.ok_or("no context")?;
    let vsb = compile("vs", "vs_5_0")?;
    let psb = compile("ps", "ps_5_0")?;
    let star = (compile("vs_star", "vs_5_0")?, compile("ps_star", "ps_5_0")?);
    let mesh = (compile("vs_mesh", "vs_5_0")?, compile("ps_mesh", "ps_5_0")?);
    Ok(Warm { device, ctx, vsb, psb, star, mesh })
}

impl Gpu {
    /// Create the device and a swap chain of `w`x`h` DIPs at `scale`, and attach it to `panel`
    /// (the XAML SwapChainPanel).
    pub fn new(panel: &windows_core::IUnknown, w: f32, h: f32, scale: (f32, f32)) -> Result<Self, String> {
        // the device and the compiled shaders, made off the window's thread at launch ([`prewarm`])
        let Warm { device, ctx, vsb, psb, star, mesh } = take_warm()?;
        let dxgi: IDXGIDevice = hr(device.cast(), "IDXGIDevice")?;
        let factory: IDXGIFactory2 = hr(unsafe { dxgi.GetAdapter().and_then(|a| a.GetParent()) }, "DXGI factory")?;

        let px = ((w * scale.0).round().max(1.0) as u32, (h * scale.1).round().max(1.0) as u32);
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: px.0,
            Height: px.1,
            Format: FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
            ..Default::default()
        };
        let swap1: IDXGISwapChain1 = hr(unsafe { factory.CreateSwapChainForComposition(&device, &desc, None) }, "CreateSwapChainForComposition")?;
        let swap: IDXGISwapChain2 = hr(swap1.cast(), "IDXGISwapChain2")?;
        hr(unsafe { swap.SetMaximumFrameLatency(1) }, "SetMaximumFrameLatency")?;
        let waitable = unsafe { swap.GetFrameLatencyWaitableObject() };
        set_inverse_scale(&swap, scale);

        // Hand the swap chain to the panel.
        unsafe {
            let native: ISwapChainPanelNative = windows_core::Interface::cast(panel).map_err(|e| format!("ISwapChainPanelNative: {e}"))?;
            native.SetSwapChain(swap.as_raw()).ok().map_err(|e| format!("SetSwapChain: {e}"))?;
        }

        // Pipeline.
        let mut vs = None;
        let mut ps = None;
        hr(unsafe { device.CreateVertexShader(&vsb, None, Some(&mut vs)) }, "CreateVertexShader")?;
        hr(unsafe { device.CreatePixelShader(&psb, None, Some(&mut ps)) }, "CreatePixelShader")?;
        let el = |name: &'static [u8], fmt: DXGI_FORMAT, slot: u32, offset: u32, per_instance: bool| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(name.as_ptr()),
            SemanticIndex: 0,
            Format: fmt,
            InputSlot: slot,
            AlignedByteOffset: offset,
            InputSlotClass: if per_instance { D3D11_INPUT_PER_INSTANCE_DATA } else { D3D11_INPUT_PER_VERTEX_DATA },
            InstanceDataStepRate: u32::from(per_instance),
        };
        let f3 = DXGI_FORMAT_R32G32B32_FLOAT;
        let f4 = DXGI_FORMAT_R32G32B32A32_FLOAT;
        let elements = [
            el(b"POSITION\0", f3, 0, 0, false),
            el(b"NORMAL\0", f3, 0, 12, false),
            el(b"RECT\0", f4, 1, 0, true),
            el(b"PARENT\0", f4, 1, 16, true),
            el(b"COLOR\0", f4, 1, 32, true),
            el(b"FX\0", f4, 1, 48, true),
        ];
        let mut layout = None;
        hr(unsafe { device.CreateInputLayout(&elements, &vsb, Some(&mut layout)) }, "CreateInputLayout")?;

        let verts = cube();
        let bytes: &[u8] = unsafe { std::slice::from_raw_parts(verts.as_ptr() as *const u8, verts.len() * 4) };
        let cube = buffer(&device, bytes.len(), D3D11_BIND_VERTEX_BUFFER, false, Some(bytes))?;
        let inst_cap = 1 << 16;
        let inst = buffer(&device, inst_cap * std::mem::size_of::<Instance>(), D3D11_BIND_VERTEX_BUFFER, true, None)?;
        let cb = buffer(&device, std::mem::size_of::<Constants>(), D3D11_BIND_CONSTANT_BUFFER, true, None)?;

        let rdesc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            DepthClipEnable: true.into(),
            MultisampleEnable: true.into(),
            AntialiasedLineEnable: false.into(),
            ..Default::default()
        };
        let mut raster = None;
        hr(unsafe { device.CreateRasterizerState(&rdesc, Some(&mut raster)) }, "CreateRasterizerState")?;
        let ddesc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: true.into(),
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
            DepthFunc: D3D11_COMPARISON_LESS,
            ..Default::default()
        };
        let mut depth_state = None;
        hr(unsafe { device.CreateDepthStencilState(&ddesc, Some(&mut depth_state)) }, "CreateDepthStencilState")?;

        // Direct2D / DirectWrite for labels.
        let d2f: ID2D1Factory1 = hr(unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }, "D2D1CreateFactory")?;
        let d2dev = hr(unsafe { d2f.CreateDevice(&dxgi) }, "ID2D1Device")?;
        let d2d = hr(unsafe { d2dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) }, "ID2D1DeviceContext")?;
        let dw: IDWriteFactory = hr(unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }, "DWriteCreateFactory")?;
        let font = windows::core::w!("Segoe UI Variable Text");
        let locale = windows::core::w!("en-us");
        let dwrite_bold = hr(
            unsafe { dw.CreateTextFormat(font, None, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 11.5, locale) },
            "CreateTextFormat",
        )?;
        let dwrite_normal = hr(
            unsafe { dw.CreateTextFormat(font, None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 10.5, locale) },
            "CreateTextFormat",
        )?;
        for f in [&dwrite_bold, &dwrite_normal] {
            unsafe {
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = f.SetTrimming(&trimming, None);
            }
        }
        let brush = |r: f32, g: f32, b: f32, a: f32| hr(unsafe { d2d.CreateSolidColorBrush(&D2D1_COLOR_F { r, g, b, a }, None) }, "brush");
        let white = brush(1.0, 1.0, 1.0, 0.94)?;
        let soft = brush(0.86, 0.89, 0.95, 0.78)?;
        let shadow = brush(0.0, 0.0, 0.0, 0.6)?;

        // Premultiplied alpha over the transparent swap chain (tiles write alpha 1).
        let mut bdesc = D3D11_BLEND_DESC::default();
        bdesc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let mut blend = None;
        hr(unsafe { device.CreateBlendState(&bdesc, Some(&mut blend)) }, "CreateBlendState")?;
        let blend = blend.ok_or("blend")?;
        // Additive for the galaxy's glowing stars.
        let mut adesc = bdesc;
        adesc.RenderTarget[0].DestBlend = D3D11_BLEND_ONE;
        adesc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ONE;
        let mut blend_add = None;
        hr(unsafe { device.CreateBlendState(&adesc, Some(&mut blend_add)) }, "CreateBlendState")?;
        let blend_add = blend_add.ok_or("blend")?;
        let off = D3D11_DEPTH_STENCIL_DESC { DepthEnable: false.into(), ..Default::default() };
        let mut depth_off = None;
        hr(unsafe { device.CreateDepthStencilState(&off, Some(&mut depth_off)) }, "CreateDepthStencilState")?;
        let depth_off = depth_off.ok_or("depth")?;
        let (svb, spb) = star;
        let (mut vs_star, mut ps_star) = (None, None);
        hr(unsafe { device.CreateVertexShader(&svb, None, Some(&mut vs_star)) }, "CreateVertexShader")?;
        hr(unsafe { device.CreatePixelShader(&spb, None, Some(&mut ps_star)) }, "CreatePixelShader")?;
        let (vs_star, ps_star) = (vs_star.ok_or("vs_star")?, ps_star.ok_or("ps_star")?);
        let cb_mirror = buffer(&device, std::mem::size_of::<Constants>(), D3D11_BIND_CONSTANT_BUFFER, true, None)?;
        let extra = buffer(&device, 1024 * std::mem::size_of::<Instance>(), D3D11_BIND_VERTEX_BUFFER, true, None)?;
        let (mvb, mpb) = mesh;
        let (mut vs_mesh, mut ps_mesh) = (None, None);
        hr(unsafe { device.CreateVertexShader(&mvb, None, Some(&mut vs_mesh)) }, "CreateVertexShader")?;
        hr(unsafe { device.CreatePixelShader(&mpb, None, Some(&mut ps_mesh)) }, "CreatePixelShader")?;
        let mesh_el = [el(b"POSITION\0", f3, 0, 0, false), el(b"COLOR\0", f4, 0, 12, false)];
        let mut layout_mesh = None;
        hr(unsafe { device.CreateInputLayout(&mesh_el, &mvb, Some(&mut layout_mesh)) }, "CreateInputLayout")?;
        let cb_mesh = buffer(&device, std::mem::size_of::<Constants>(), D3D11_BIND_CONSTANT_BUFFER, true, None)?;
        let mesh_cap = 1 << 16;
        let mesh_buf = buffer(&device, mesh_cap * std::mem::size_of::<Vertex>(), D3D11_BIND_VERTEX_BUFFER, true, None)?;

        let mut gpu = Gpu {
            device,
            ctx,
            swap,
            waitable,
            vs: vs.ok_or("vs")?,
            ps: ps.ok_or("ps")?,
            layout: layout.ok_or("layout")?,
            cube,
            cube_count: (verts.len() / 6) as u32,
            inst,
            inst_cap,
            cb,
            raster: raster.ok_or("raster")?,
            depth_state: depth_state.ok_or("depth")?,
            blend,
            blend_add,
            depth_off,
            vs_star,
            ps_star,
            cb_mirror,
            extra,
            vs_mesh: vs_mesh.ok_or("vs_mesh")?,
            ps_mesh: ps_mesh.ok_or("ps_mesh")?,
            layout_mesh: layout_mesh.ok_or("layout_mesh")?,
            cb_mesh,
            mesh_buf,
            mesh_cap,
            msaa: None,
            samples: 4,
            d2d,
            dwrite_bold,
            dwrite_normal,
            white,
            soft,
            shadow,
            px,
            scale,
            want_capture: false,
            captured: None,
        };
        gpu.make_targets()?;
        Ok(gpu)
    }

    /// Antialiasing: 1 (off), 2, 4 or 8 samples per pixel. Falls back to the most the GPU supports.
    pub fn set_samples(&mut self, want: u32) -> Result<(), String> {
        let mut n = want.clamp(1, 8).next_power_of_two();
        while n > 1 {
            let ok = |fmt| unsafe { self.device.CheckMultisampleQualityLevels(fmt, n) }.is_ok_and(|q| q > 0);
            if ok(FORMAT) && ok(DXGI_FORMAT_D24_UNORM_S8_UINT) {
                break;
            }
            n /= 2;
        }
        if n == self.samples && self.msaa.is_some() {
            return Ok(());
        }
        self.samples = n;
        self.make_targets()
    }

    fn make_targets(&mut self) -> Result<(), String> {
        self.msaa = None;
        let (w, h) = self.px;
        let tex = |fmt: DXGI_FORMAT, bind: D3D11_BIND_FLAG| -> Result<ID3D11Texture2D, String> {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: fmt,
                SampleDesc: DXGI_SAMPLE_DESC { Count: self.samples, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: bind.0 as u32,
                ..Default::default()
            };
            let mut t = None;
            hr(unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut t)) }, "CreateTexture2D")?;
            t.ok_or_else(|| "texture".into())
        };
        let color = tex(FORMAT, D3D11_BIND_RENDER_TARGET)?;
        let depth = tex(DXGI_FORMAT_D24_UNORM_S8_UINT, D3D11_BIND_DEPTH_STENCIL)?;
        let mut rtv = None;
        let mut dsv = None;
        hr(unsafe { self.device.CreateRenderTargetView(&color, None, Some(&mut rtv)) }, "CreateRenderTargetView")?;
        hr(unsafe { self.device.CreateDepthStencilView(&depth, None, Some(&mut dsv)) }, "CreateDepthStencilView")?;
        self.msaa = Some((color, rtv.ok_or("rtv")?, dsv.ok_or("dsv")?));
        Ok(())
    }

    /// New panel size (DIPs) or composition scale.
    pub fn resize(&mut self, w: f32, h: f32, scale: (f32, f32)) -> Result<(), String> {
        let px = ((w * scale.0).round().max(1.0) as u32, (h * scale.1).round().max(1.0) as u32);
        if px == self.px && scale == self.scale && self.msaa.is_some() && self.swap_size() == px {
            return Ok(());
        }
        // ResizeBuffers fails while anything still holds the old buffers: Direct2D's target, a
        // view bound to the context, or a view already let go of but not yet destroyed (D3D11
        // destroys them when the context next flushes). So: unbind, clear, flush, then resize.
        unsafe {
            self.d2d.SetTarget(None);
            self.ctx.OMSetRenderTargets(None, None);
            self.ctx.ClearState();
        }
        self.msaa = None;
        unsafe { self.ctx.Flush() };
        let resized = if dev_resize_fails() {
            Err("ResizeBuffers: failed on purpose (SPRAWL_RESIZE_FAIL)".to_string())
        } else {
            hr(
                unsafe { self.swap.ResizeBuffers(2, px.0, px.1, FORMAT, DXGI_SWAP_CHAIN_FLAG(DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0)) },
                "ResizeBuffers",
            )
        };
        if resized.is_ok() {
            self.px = px;
            self.scale = scale;
            set_inverse_scale(&self.swap, scale);
        }
        // the targets again at the size the swap chain has now, so the map keeps drawing even
        // when the resize failed (the caller tries it again)
        self.make_targets()?;
        resized?;
        let got = self.swap_size();
        if got != px {
            return Err(format!("the swap chain is {}x{} after resizing it to {}x{}", got.0, got.1, px.0, px.1));
        }
        Ok(())
    }

    /// True when the GPU was lost (a driver update or reset, the adapter removed): everything
    /// made on it must be made again.
    pub fn lost(&self) -> bool {
        unsafe { self.device.GetDeviceRemovedReason() }.is_err()
    }

    /// The swap chain's buffers as DXGI has them, in pixels.
    pub fn swap_size(&self) -> (u32, u32) {
        unsafe { self.swap.GetDesc1() }.map(|d| (d.Width, d.Height)).unwrap_or((0, 0))
    }

    /// True when the swap chain can take another frame without blocking.
    pub fn ready(&self) -> bool {
        unsafe { WaitForSingleObject(self.waitable, 0) == WAIT_OBJECT_0 }
    }

    pub fn render(&mut self, f: &Frame) -> Result<(), String> {
        let Some((msaa_tex, rtv, dsv)) = self.msaa.clone() else { return Ok(()) };
        let ctx = &self.ctx;
        let need = f.instances.len().max(f.stars.len());
        if need > self.inst_cap {
            self.inst_cap = need.next_power_of_two();
            self.inst = buffer(&self.device, self.inst_cap * std::mem::size_of::<Instance>(), D3D11_BIND_VERTEX_BUFFER, true, None)?;
        }
        unsafe {
            let consts = Constants {
                vp: f.vp,
                eye: [f.eye[0], f.eye[1], f.eye[2], f.time],
                light: [f.light[0], f.light[1], f.light[2], 0.0],
                size: [f.size[0], f.size[1], 0.0, f.cushion],
                right: [f.right[0], f.right[1], f.right[2], 0.0],
                up: [f.up[0], f.up[1], f.up[2], 0.0],
            };
            write(ctx, &self.cb, std::slice::from_ref(&consts))?;
            let mirror = Constants { size: [f.size[0], f.size[1], 1.0, f.cushion], ..consts };
            write(ctx, &self.cb_mirror, std::slice::from_ref(&mirror))?;
            if !f.instances.is_empty() {
                write(ctx, &self.inst, f.instances)?;
            }

            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), &dsv);
            ctx.ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 0.0]);
            ctx.ClearDepthStencilView(&dsv, D3D11_CLEAR_DEPTH.0 as u32, 1.0, 0);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: self.px.0 as f32, Height: self.px.1 as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
            ctx.RSSetState(&self.raster);
            ctx.OMSetDepthStencilState(&self.depth_state, 0);
            ctx.IASetInputLayout(&self.layout);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.OMSetBlendState(&self.blend, None, 0xffff_ffff);
            ctx.PSSetShader(&self.ps, None);
            ctx.VSSetShader(&self.vs, None);
            let n = f.instances.len();
            if f.mirror && n > 0 {
                self.pass(&self.cb_mirror, &self.inst, n);
            }
            if let Some(fl) = f.floor {
                write(ctx, &self.extra, std::slice::from_ref(&fl))?;
                self.pass(&self.cb, &self.extra, 1);
            }
            if n > 0 {
                self.pass(&self.cb, &self.inst, n);
            }
            if !f.stars.is_empty() {
                write(ctx, &self.inst, f.stars)?;
                ctx.OMSetBlendState(&self.blend_add, None, 0xffff_ffff);
                ctx.OMSetDepthStencilState(&self.depth_off, 0);
                ctx.VSSetShader(&self.vs_star, None);
                ctx.PSSetShader(&self.ps_star, None);
                // the first face of the cube is a unit quad: enough for a billboard
                let bufs = [Some(self.cube.clone()), Some(self.inst.clone())];
                let strides = [24u32, std::mem::size_of::<Instance>() as u32];
                let offsets = [0u32, 0];
                ctx.IASetVertexBuffers(0, 2, Some(bufs.as_ptr()), Some(strides.as_ptr()), Some(offsets.as_ptr()));
                ctx.VSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
                ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
                ctx.DrawInstanced(6, f.stars.len() as u32, 0, 0);
            }
            let (nt, nl) = (f.mesh.len(), f.lines.len());
            if nt + nl > 0 {
                if nt + nl > self.mesh_cap {
                    self.mesh_cap = (nt + nl).next_power_of_two();
                    self.mesh_buf = buffer(&self.device, self.mesh_cap * std::mem::size_of::<Vertex>(), D3D11_BIND_VERTEX_BUFFER, true, None)?;
                }
                let mut all = Vec::with_capacity(nt + nl);
                all.extend_from_slice(f.mesh);
                all.extend_from_slice(f.lines);
                write(ctx, &self.mesh_buf, &all)?;
                let mc = Constants { right: [f.right[0], f.right[1], f.right[2], if f.mesh_2d { 1.0 } else { 0.0 }], ..consts };
                write(ctx, &self.cb_mesh, std::slice::from_ref(&mc))?;
                ctx.OMSetBlendState(&self.blend, None, 0xffff_ffff);
                ctx.OMSetDepthStencilState(if f.mesh_2d { &self.depth_off } else { &self.depth_state }, 0);
                ctx.IASetInputLayout(&self.layout_mesh);
                ctx.VSSetShader(&self.vs_mesh, None);
                ctx.PSSetShader(&self.ps_mesh, None);
                ctx.VSSetConstantBuffers(0, Some(&[Some(self.cb_mesh.clone())]));
                ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb_mesh.clone())]));
                let stride = std::mem::size_of::<Vertex>() as u32;
                ctx.IASetVertexBuffers(0, 1, Some(&Some(self.mesh_buf.clone())), Some(&stride), Some(&0));
                if nt > 0 {
                    ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                    ctx.Draw(nt as u32, 0);
                }
                if nl > 0 {
                    ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_LINELIST);
                    ctx.Draw(nl as u32, nt as u32);
                }
            }

            let back: ID3D11Texture2D = hr(self.swap.GetBuffer(0), "GetBuffer")?;
            if self.samples > 1 {
                ctx.ResolveSubresource(&back, 0, &msaa_tex, 0, FORMAT);
            } else {
                ctx.CopyResource(&back, &msaa_tex);
            }
            ctx.OMSetRenderTargets(None, None);

            // Labels.
            if !f.labels.is_empty() {
                let surface: IDXGISurface = hr(back.cast(), "IDXGISurface")?;
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: FORMAT, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0 * self.scale.0,
                    dpiY: 96.0 * self.scale.1,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    colorContext: std::mem::ManuallyDrop::new(None),
                };
                let target = hr(self.d2d.CreateBitmapFromDxgiSurface(&surface, Some(&props)), "CreateBitmapFromDxgiSurface")?;
                self.d2d.SetTarget(&target);
                self.d2d.SetDpi(96.0 * self.scale.0, 96.0 * self.scale.1);
                self.d2d.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                self.d2d.BeginDraw();
                for l in f.labels {
                    let text: Vec<u16> = l.text.encode_utf16().collect();
                    let fmt = if l.strong { &self.dwrite_bold } else { &self.dwrite_normal };
                    let brush = if l.strong { &self.white } else { &self.soft };
                    let r = D2D_RECT_F { left: l.x + 1.0, top: l.y + 1.0, right: l.x + 1.0 + l.max_w, bottom: l.y + 20.0 };
                    self.d2d.DrawText(&text, fmt, &r, &self.shadow, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
                    let r = D2D_RECT_F { left: l.x, top: l.y, right: l.x + l.max_w, bottom: l.y + 20.0 };
                    self.d2d.DrawText(&text, fmt, &r, brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
                }
                let _ = self.d2d.EndDraw(None, None);
                self.d2d.SetTarget(None);
                let _ = std::mem::ManuallyDrop::into_inner(props.colorContext);
            }
            if self.want_capture {
                self.want_capture = false;
                self.captured = self.read_back(&back).ok();
            }
            let _ = self.swap.Present(1, DXGI_PRESENT(0));
        }
        Ok(())
    }

    /// Draw `n` instances from `buf` with constants `cb`.
    fn pass(&self, cb: &ID3D11Buffer, buf: &ID3D11Buffer, n: usize) {
        let ctx = &self.ctx;
        let bufs = [Some(self.cube.clone()), Some(buf.clone())];
        let strides = [24u32, std::mem::size_of::<Instance>() as u32];
        let offsets = [0u32, 0];
        unsafe {
            ctx.IASetVertexBuffers(0, 2, Some(bufs.as_ptr()), Some(strides.as_ptr()), Some(offsets.as_ptr()));
            ctx.VSSetConstantBuffers(0, Some(&[Some(cb.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(cb.clone())]));
            ctx.DrawInstanced(self.cube_count, n as u32, 0, 0);
        }
    }

    fn read_back(&self, back: &ID3D11Texture2D) -> Result<(u32, u32, Vec<u8>), String> {
        let (w, h) = self.px;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        let mut staging = None;
        hr(unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut staging)) }, "staging texture")?;
        let staging: ID3D11Texture2D = staging.ok_or("staging")?;
        unsafe {
            self.ctx.CopyResource(&staging, back);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            hr(self.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m)), "Map staging")?;
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), w as usize * 4);
                out.extend_from_slice(row);
            }
            self.ctx.Unmap(&staging, 0);
            Ok((w, h, out))
        }
    }

}

/// dev: `SPRAWL_RESIZE_FAIL=n` makes the swap chain's first n resizes fail, to see the map
/// recover from it.
fn dev_resize_fails() -> bool {
    thread_local! {
        static LEFT: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    }
    LEFT.with(|l| {
        let n = l.get().unwrap_or_else(|| std::env::var("SPRAWL_RESIZE_FAIL").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
        l.set(Some(n.saturating_sub(1)));
        n > 0
    })
}

fn set_inverse_scale(swap: &IDXGISwapChain2, scale: (f32, f32)) {
    let m = DXGI_MATRIX_3X2_F { _11: 1.0 / scale.0, _12: 0.0, _21: 0.0, _22: 1.0 / scale.1, _31: 0.0, _32: 0.0 };
    unsafe {
        let _ = swap.SetMatrixTransform(&m);
    }
}

unsafe fn write<T: Copy>(ctx: &ID3D11DeviceContext, buf: &ID3D11Buffer, data: &[T]) -> Result<(), String> {
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    hr(unsafe { ctx.Map(buf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped)) }, "Map")?;
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, mapped.pData as *mut u8, std::mem::size_of_val(data));
        ctx.Unmap(buf, 0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Every shader compiles (a mistake in the HLSL otherwise shows only as a map that can't start).
    #[test]
    fn shaders_compile() {
        for (entry, target) in [("vs", "vs_5_0"), ("ps", "ps_5_0"), ("vs_star", "vs_5_0"), ("ps_star", "ps_5_0"), ("vs_mesh", "vs_5_0"), ("ps_mesh", "ps_5_0")] {
            if let Err(e) = super::compile(entry, target) {
                panic!("{entry}: {e}");
            }
        }
    }
}
