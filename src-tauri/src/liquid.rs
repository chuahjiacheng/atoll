//! Liquid glass under the island: real refraction of the desktop.
//!
//! Windows' own backdrop brush (backdrop.rs, "Frosted") hands us the desktop
//! already blurred by a fixed amount, and no composition effect can bend it.
//! For glass that bends light at its rim, the renderer needs the desktop's
//! pixels, so this captures the island's screen (Windows.Graphics.Capture) and
//! draws the glass itself with a small pixel shader:
//!
//! - blur: any amount, 0 (clear glass) up to 48 px, from a mip chain and a
//!   16-tap disc;
//! - distortion: a bevel around the rim that bends what lies just outside the
//!   island into its edge, like the thick edge of a lens;
//! - liquid glass: a lit rim, a soft inner glow, a little colour split at the
//!   bend and a saturation lift.
//!
//! The island and its helper window are kept out of the capture
//! (WDA_EXCLUDEFROMCAPTURE), or the glass would refract itself. Side effect:
//! while Liquid is on, the island doesn't show in screenshots or recordings
//! (dev builds have a tray toggle that freezes the glass and lifts this).
//! The output is a composition swap chain shown in backdrop.rs's helper window.

use std::sync::{Arc, Mutex};

use windows::core::{Interface, Ref, PCSTR};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

/// Desktop kept around the island window, so the rim can bend in what lies
/// beyond its edge and the blur has something to pull from.
const MARGIN: i32 = 96;

/// What the glass looks like (from Settings → Appearance), plus where it is.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Look {
    /// Island shape in px inside the helper window: x, y, w, h, corner radius.
    pub shape: [f32; 5],
    /// Blur radius, px.
    pub blur: f32,
    /// 0..1
    pub distortion: f32,
    /// 0..1: rim light, inner glow, colour split, saturation.
    pub liquid: f32,
    /// Hull colour (0xRRGGBB) and how much of it tints the glass.
    pub tint: u32,
    pub tint_alpha: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Constants {
    shape: [f32; 4],
    p0: [f32; 4],
    p1: [f32; 4],
    tint: [f32; 4],
    src: [f32; 4],
}

const SHADER: &str = r#"
cbuffer P : register(b0) {
  float4 shape;  // x y w h, px in the window
  float4 p0;     // radius, blur px, distortion 0..1, liquid 0..1
  float4 p1;     // tint alpha, unused, window -> source offset x y
  float4 tint;   // rgb
  float4 src;    // source w h, last mip
};
Texture2D tex : register(t0);
SamplerState smp : register(s0);

float4 vs(uint id : SV_VertexID) : SV_Position {
  float2 uv = float2((id << 1) & 2, id & 2);
  return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}

float3 fetch(float2 p, float lod) {
  return tex.SampleLevel(smp, (p + p1.zw) / src.xy, lod).rgb;
}

float3 blurred(float2 p, float radius) {
  float3 acc = fetch(p, 0);
  if (radius >= 0.75) {
    float lod = clamp(log2(radius) - 1.5, 0, src.z);
    acc = 0;
    [unroll] for (int i = 0; i < 16; i++) {
      float r = sqrt((i + 0.5) / 16.0) * radius;
      float a = i * 2.39996323;
      acc += fetch(p + float2(cos(a), sin(a)) * r, lod);
    }
    acc /= 16.0;
  }
  return acc;
}

float4 ps(float4 pos : SV_Position) : SV_Target {
  float2 p = pos.xy;
  float2 hs = shape.zw * 0.5;
  float2 rel = p - (shape.xy + hs);
  float r = min(p0.x, min(hs.x, hs.y));
  float2 q = abs(rel) - (hs - r);
  float d = length(max(q, 0)) + min(max(q.x, q.y), 0) - r;   // < 0 inside
  float alpha = saturate(0.5 - d);
  if (alpha <= 0) return 0;

  float2 sg = float2(rel.x < 0 ? -1 : 1, rel.y < 0 ? -1 : 1);
  float2 n = (q.x > 0 && q.y > 0) ? normalize(q) * sg
           : (q.x > q.y ? float2(sg.x, 0) : float2(0, sg.y));  // outward normal

  float distortion = p0.z, liquid = p0.w;
  float band = min(min(hs.x, hs.y), 10 + 30 * distortion);    // bevel width
  float t = saturate(-d / max(band, 1));                      // 0 at the rim, 1 past the bevel
  float bend = pow(1 - t, 2.2);
  float mag = distortion * band * bend;                       // px pulled in from outside
  float split = liquid * 0.1 * mag;

  float3 col = blurred(p + n * mag, p0.y);
  if (split > 0.4) {
    col.r = blurred(p + n * (mag + split), p0.y).r;
    col.b = blurred(p + n * max(mag - split, 0), p0.y).b;
  }

  // Saturation lift, then the hull tint.
  float lum = dot(col, float3(0.2126, 0.7152, 0.0722));
  col = lerp(lum.xxx, col, 1 + 0.45 * liquid);
  col = lerp(col, tint.rgb, p1.x);

  // Light from the top left: a bright lit rim, a dimmer one opposite, and a
  // soft glow along the inside of the bevel.
  float2 L = normalize(float2(-0.55, -1));
  float facing = dot(n, L);
  float rim = exp(min(d, 0) / 1.1);
  float spec = rim * (0.25 + 1.1 * pow(saturate(facing), 3) + 0.5 * pow(saturate(-facing), 3));
  float glow = pow(1 - t, 3) * (0.35 + 0.65 * saturate(facing));
  col += liquid * (0.6 * spec + 0.14 * glow);

  col = saturate(col);
  return float4(col * alpha, alpha);
}
"#;

struct Source {
    tex: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    /// Screen rectangle the texture holds.
    rect: (i32, i32, i32, i32),
    mips: u32,
}

struct Capture {
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    monitor: isize,
    size: SizeInt32,
}

struct Inner {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
    swap: IDXGISwapChain1,
    rtv: Option<ID3D11RenderTargetView>,
    size: (u32, u32),
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cbuf: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    source: Option<Source>,
    look: Look,
    /// The helper window's screen rectangle (x, y, w, h).
    window: (i32, i32, i32, i32),
    /// Top-left of the captured monitor, in screen px.
    monitor_origin: (i32, i32),
    capture: Option<Capture>,
}

/// The renderer. Cheap to clone; the capture thread holds a clone.
#[derive(Clone)]
pub struct Liquid(Arc<Mutex<Inner>>);

// The D3D objects are free-threaded (multithread protection is on) and every
// use goes through the mutex.
unsafe impl Send for Inner {}

unsafe fn compile(entry: &str, target: &str) -> windows::core::Result<ID3DBlob> {
    let entry = std::ffi::CString::new(entry).unwrap();
    let target = std::ffi::CString::new(target).unwrap();
    let mut code = None;
    let mut errors = None;
    let result = D3DCompile(
        SHADER.as_ptr().cast(),
        SHADER.len(),
        PCSTR::null(),
        None,
        None,
        PCSTR(entry.as_ptr().cast()),
        PCSTR(target.as_ptr().cast()),
        0,
        0,
        &mut code,
        Some(&mut errors),
    );
    if let Some(e) = errors {
        let text = std::slice::from_raw_parts(e.GetBufferPointer() as *const u8, e.GetBufferSize());
        eprintln!("[atoll] liquid glass shader: {}", String::from_utf8_lossy(text));
    }
    result?;
    Ok(code.unwrap())
}

fn blob(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

impl Liquid {
    /// Device, shaders and a swap chain (`w` × `h`) for composition.
    pub fn new(w: u32, h: u32) -> windows::core::Result<Liquid> {
        unsafe {
            let mut device = None;
            let mut ctx = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )?;
            let device: ID3D11Device = device.unwrap();
            let ctx: ID3D11DeviceContext = ctx.unwrap();
            let _ = device.cast::<ID3D11Multithread>()?.SetMultithreadProtected(true);
            let dxgi: IDXGIDevice = device.cast()?;
            let winrt_device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w.max(1),
                Height: h.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForComposition(&device, &desc, None)?;

            let vsb = compile("vs", "vs_5_0")?;
            let psb = compile("ps", "ps_5_0")?;
            let mut vs = None;
            device.CreateVertexShader(blob(&vsb), None, Some(&mut vs))?;
            let mut ps = None;
            device.CreatePixelShader(blob(&psb), None, Some(&mut ps))?;
            let mut cbuf = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: std::mem::size_of::<Constants>() as u32,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut cbuf),
            )?;
            let mut sampler = None;
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                },
                Some(&mut sampler),
            )?;
            let inner = Inner {
                device,
                ctx,
                winrt_device,
                swap,
                rtv: None,
                size: (w.max(1), h.max(1)),
                vs: vs.unwrap(),
                ps: ps.unwrap(),
                cbuf: cbuf.unwrap(),
                sampler: sampler.unwrap(),
                source: None,
                look: Look::default(),
                window: (0, 0, 0, 0),
                monitor_origin: (0, 0),
                capture: None,
            };
            Ok(Liquid(Arc::new(Mutex::new(inner))))
        }
    }

    pub fn swap_chain(&self) -> IDXGISwapChain1 {
        self.0.lock().unwrap().swap.clone()
    }

    /// Start (or keep) capturing `monitor` whose top-left is `origin`.
    pub fn capture(&self, monitor: isize, origin: (i32, i32)) -> windows::core::Result<()> {
        let mut g = self.0.lock().unwrap();
        g.monitor_origin = origin;
        if g.capture.as_ref().is_some_and(|c| c.monitor == monitor) {
            return Ok(());
        }
        g.capture = None;
        g.source = None;
        unsafe {
            let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = interop.CreateForMonitor(HMONITOR(monitor as _))?;
            let size = item.Size()?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &g.winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )?;
            let me = self.clone();
            pool.FrameArrived(&TypedEventHandler::new(move |pool: Ref<Direct3D11CaptureFramePool>, _| {
                if let Some(pool) = pool.as_ref() {
                    me.on_frame(pool);
                }
                Ok(())
            }))?;
            let session = pool.CreateCaptureSession(&item)?;
            let _ = session.SetIsCursorCaptureEnabled(false);
            // Windows 11 lets a desktop app drop the yellow capture border.
            let _ = session.SetIsBorderRequired(false);
            session.StartCapture()?;
            g.capture = Some(Capture { _item: item, pool, session, monitor, size });
        }
        Ok(())
    }

    pub fn stop(&self) {
        let mut g = self.0.lock().unwrap();
        if let Some(c) = g.capture.take() {
            let _ = c.session.Close();
            let _ = c.pool.Close();
        }
        g.source = None;
    }

    /// New shape/look or window position: redraw from the last frame.
    /// Returns whether a frame was presented (which waits for the screen).
    pub fn set(&self, look: Look, window: (i32, i32, i32, i32)) -> bool {
        let mut g = self.0.lock().unwrap();
        if g.look == look && g.window == window {
            return false;
        }
        g.look = look;
        g.window = window;
        g.source.is_some() && unsafe { g.draw().is_ok() }
    }

    fn on_frame(&self, pool: &Direct3D11CaptureFramePool) {
        let Ok(frame) = pool.TryGetNextFrame() else { return };
        let mut g = self.0.lock().unwrap();
        unsafe {
            let Ok(size) = frame.ContentSize() else { return };
            if let Some(c) = g.capture.as_mut() {
                if c.size.Width != size.Width || c.size.Height != size.Height {
                    c.size = size;
                    let _ = pool.Recreate(&g.winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size);
                    return;
                }
            }
            let Ok(surface) = frame.Surface() else { return };
            let Ok(access) = surface.cast::<IDirect3DDxgiInterfaceAccess>() else { return };
            let Ok(frame_tex) = access.GetInterface::<ID3D11Texture2D>() else { return };
            let _ = g.copy_frame(&frame_tex, size);
            let _ = g.draw();
        }
        let _ = frame.Close();
    }
}

impl Inner {
    /// Copy the island window's surroundings out of a captured monitor frame.
    unsafe fn copy_frame(&mut self, frame: &ID3D11Texture2D, size: SizeInt32) -> windows::core::Result<()> {
        let (wx, wy, ww, wh) = self.window;
        if ww <= 0 || wh <= 0 {
            return Ok(());
        }
        let (mx, my) = self.monitor_origin;
        // Window ± margin, in monitor px, clamped to the monitor.
        let left = (wx - MARGIN - mx).clamp(0, size.Width);
        let top = (wy - MARGIN - my).clamp(0, size.Height);
        let right = (wx + ww + MARGIN - mx).clamp(0, size.Width);
        let bottom = (wy + wh + MARGIN - my).clamp(0, size.Height);
        let (w, h) = (right - left, bottom - top);
        if w <= 0 || h <= 0 {
            return Ok(());
        }
        let rect = (left + mx, top + my, w, h);
        let fits = self.source.as_ref().is_some_and(|s| s.rect.2 == w && s.rect.3 == h);
        if !fits {
            let mut tex = None;
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: w as u32,
                    Height: h as u32,
                    MipLevels: 0,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
                },
                None,
                Some(&mut tex),
            )?;
            let tex = tex.unwrap();
            let mut srv = None;
            self.device.CreateShaderResourceView(&tex, None, Some(&mut srv))?;
            let mips = 32 - (w.max(h) as u32).leading_zeros();
            self.source = Some(Source { tex, srv: srv.unwrap(), rect, mips });
        }
        let source = self.source.as_mut().unwrap();
        source.rect = rect;
        let bx = D3D11_BOX { left: left as u32, top: top as u32, front: 0, right: right as u32, bottom: bottom as u32, back: 1 };
        self.ctx.CopySubresourceRegion(&source.tex, 0, 0, 0, 0, frame, 0, Some(&bx));
        self.ctx.GenerateMips(&source.srv);
        Ok(())
    }

    unsafe fn draw(&mut self) -> windows::core::Result<()> {
        let Some(source) = self.source.as_ref() else { return Ok(()) };
        let (wx, wy, ww, wh) = self.window;
        if ww <= 0 || wh <= 0 {
            return Ok(());
        }
        let (w, h) = (ww as u32, wh as u32);
        if self.size != (w, h) || self.rtv.is_none() {
            self.rtv = None;
            if self.size != (w, h) {
                self.swap.ResizeBuffers(2, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SWAP_CHAIN_FLAG(0))?;
                self.size = (w, h);
            }
            let back: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            let mut rtv = None;
            self.device.CreateRenderTargetView(&back, None, Some(&mut rtv))?;
            self.rtv = rtv;
        }
        let l = self.look;
        let c = Constants {
            shape: [l.shape[0], l.shape[1], l.shape[2], l.shape[3]],
            p0: [l.shape[4], l.blur, l.distortion, l.liquid],
            p1: [l.tint_alpha, 0.0, (wx - source.rect.0) as f32, (wy - source.rect.1) as f32],
            tint: [
                ((l.tint >> 16) & 0xff) as f32 / 255.0,
                ((l.tint >> 8) & 0xff) as f32 / 255.0,
                (l.tint & 0xff) as f32 / 255.0,
                0.0,
            ],
            src: [source.rect.2 as f32, source.rect.3 as f32, source.mips.saturating_sub(1) as f32, 0.0],
        };
        let ctx = &self.ctx;
        ctx.UpdateSubresource(&self.cbuf, 0, None, (&c as *const Constants).cast(), 0, 0);
        let rtv = self.rtv.clone();
        ctx.OMSetRenderTargets(Some(&[rtv.clone()]), None);
        ctx.ClearRenderTargetView(rtv.as_ref().unwrap(), &[0.0; 4]);
        ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: w as f32,
            Height: h as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ctx.IASetInputLayout(None);
        ctx.VSSetShader(&self.vs, None);
        ctx.PSSetShader(&self.ps, None);
        ctx.PSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
        ctx.PSSetShaderResources(0, Some(&[Some(source.srv.clone())]));
        ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
        ctx.Draw(3, 0);
        // Unbind so GenerateMips can write the source next frame.
        ctx.PSSetShaderResources(0, Some(&[None]));
        ctx.OMSetRenderTargets(None, None);
        self.swap.Present(1, DXGI_PRESENT(0)).ok()
    }
}

impl Liquid {
    /// Stop capturing but keep the last frame, so the glass can be shown
    /// without hiding it from capture (it must not see itself). Used by the
    /// probe test and by the dev-build screenshot mode (backdrop.rs).
    pub fn freeze(&self) {
        if let Some(c) = self.0.lock().unwrap().capture.take() {
            let _ = c.session.Close();
            let _ = c.pool.Close();
        }
    }

    pub fn has_frame(&self) -> bool {
        self.0.lock().unwrap().source.is_some()
    }
}
