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

const SAMPLES: u32 = 4;
const FORMAT: DXGI_FORMAT = DXGI_FORMAT_B8G8R8A8_UNORM;

const HLSL: &str = r#"
cbuffer Frame : register(b0) { float4x4 uVP; float4 uEye; float4 uLight; float4 uSize; };
struct VSIn { float3 pos : POSITION; float3 nor : NORMAL; float4 rect : RECT; float4 par : PARENT; float4 col : COLOR; float4 fx : FX; };
struct PSIn { float4 sv : SV_Position; float3 n : NORMAL; float2 uv : UV; float2 puv : PUV; float4 col : COLOR; float4 fx : FX; float3 w : WORLD; float2 wh : WH; };
PSIn vs(VSIn i) {
    PSIn o;
    float2 xy = i.rect.xy + i.pos.xy * i.rect.zw;
    float h = i.fx.x + i.fx.y;
    float3 w = float3(xy.x - uSize.x * 0.5, uSize.y * 0.5 - xy.y, i.pos.z * h);
    o.sv = mul(uVP, float4(w, 1.0));
    o.n = float3(i.nor.x, -i.nor.y, i.nor.z);
    o.uv = i.pos.xy;
    o.puv = (xy - i.par.xy) / max(i.par.zw, 1e-3);
    o.col = i.col; o.fx = i.fx; o.w = w; o.wh = i.rect.zw;
    return o;
}
float4 ps(PSIn i) : SV_Target {
    float3 base = i.col.rgb; float kind = i.col.a; bool top = i.n.z > 0.5;
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
    return float4(c, 1.0);
}
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
    pub instances: &'a [Instance],
    pub labels: &'a [Label],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Constants {
    vp: [f32; 16],
    eye: [f32; 4],
    light: [f32; 4],
    size: [f32; 4],
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
    msaa: Option<(ID3D11Texture2D, ID3D11RenderTargetView, ID3D11DepthStencilView)>,
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

impl Gpu {
    /// Create the device and a swap chain of `w`x`h` DIPs at `scale`, and attach it to `panel`
    /// (the XAML SwapChainPanel).
    pub fn new(panel: &windows_core::IUnknown, w: f32, h: f32, scale: (f32, f32)) -> Result<Self, String> {
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
        let vsb = compile("vs", "vs_5_0")?;
        let psb = compile("ps", "ps_5_0")?;
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
            msaa: None,
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
                SampleDesc: DXGI_SAMPLE_DESC { Count: SAMPLES, Quality: 0 },
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
        if px == self.px && scale == self.scale {
            return Ok(());
        }
        unsafe { self.d2d.SetTarget(None) };
        self.msaa = None;
        unsafe { self.ctx.ClearState() };
        hr(
            unsafe { self.swap.ResizeBuffers(2, px.0, px.1, FORMAT, DXGI_SWAP_CHAIN_FLAG(DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0)) },
            "ResizeBuffers",
        )?;
        self.px = px;
        self.scale = scale;
        set_inverse_scale(&self.swap, scale);
        self.make_targets()
    }

    /// True when the swap chain can take another frame without blocking.
    pub fn ready(&self) -> bool {
        unsafe { WaitForSingleObject(self.waitable, 0) == WAIT_OBJECT_0 }
    }

    pub fn render(&mut self, f: &Frame) -> Result<(), String> {
        let Some((msaa_tex, rtv, dsv)) = self.msaa.clone() else { return Ok(()) };
        let ctx = &self.ctx;
        if f.instances.len() > self.inst_cap {
            self.inst_cap = f.instances.len().next_power_of_two();
            self.inst = buffer(&self.device, self.inst_cap * std::mem::size_of::<Instance>(), D3D11_BIND_VERTEX_BUFFER, true, None)?;
        }
        unsafe {
            let consts = Constants {
                vp: f.vp,
                eye: [f.eye[0], f.eye[1], f.eye[2], 1.0],
                light: [f.light[0], f.light[1], f.light[2], 0.0],
                size: [f.size[0], f.size[1], 0.0, f.cushion],
            };
            write(ctx, &self.cb, std::slice::from_ref(&consts))?;
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
            let bufs = [Some(self.cube.clone()), Some(self.inst.clone())];
            let strides = [24u32, std::mem::size_of::<Instance>() as u32];
            let offsets = [0u32, 0];
            ctx.IASetVertexBuffers(0, 2, Some(bufs.as_ptr()), Some(strides.as_ptr()), Some(offsets.as_ptr()));
            ctx.VSSetShader(&self.vs, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            if !f.instances.is_empty() {
                ctx.DrawInstanced(self.cube_count, f.instances.len() as u32, 0, 0);
            }

            let back: ID3D11Texture2D = hr(self.swap.GetBuffer(0), "GetBuffer")?;
            ctx.ResolveSubresource(&back, 0, &msaa_tex, 0, FORMAT);
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
