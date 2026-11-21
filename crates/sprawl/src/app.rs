//! Windows App Runtime start-up and the composed XAML `Application`.

use std::sync::OnceLock;

use windows_core::{implement, Interface, Ref, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::XamlControlsResources;
use crate::bindings::Microsoft::UI::Xaml::Markup::{IXamlMetadataProvider, IXamlMetadataProvider_Impl, IXamlType, XmlnsDefinition};
use crate::bindings::Microsoft::UI::Xaml::Media::Imaging::WriteableBitmap;
use crate::bindings::Microsoft::UI::Xaml::XamlTypeInfo::XamlControlsXamlMetaDataProvider;
use crate::bindings::Microsoft::UI::Xaml::{
    Application, IApplicationOverrides, IApplicationOverrides_Impl, LaunchActivatedEventArgs, ResourceDictionary, Window,
};
use crate::bindings::Windows::UI::Xaml::Interop::TypeName;

// ---------------------------------------------------------------- Windows App Runtime start-up

#[cfg(has_bootstrap)]
pub(crate) mod bootstrap {
    #[link(name = "Microsoft.WindowsAppRuntime.Bootstrap")]
    extern "system" {
        fn MddBootstrapInitialize2(major_minor: u32, version_tag: *const u16, min_version: u64, options: u32) -> i32;
    }
    /// Windows App SDK 1.8; show the "install the runtime" prompt when it's missing.
    pub fn start() -> Result<(), i32> {
        let hr = unsafe { MddBootstrapInitialize2(0x0001_0008, std::ptr::null(), 0, 0x18) };
        if hr < 0 { Err(hr) } else { Ok(()) }
    }
}

#[windows_core::interface("EECDBF0E-BAE9-4CB6-A68E-9598E1CB57BB")]
unsafe trait IWindowNative: windows_core::IUnknown {
    fn WindowHandle(&self, hwnd: *mut isize) -> windows_core::HRESULT;
}

#[windows_core::interface("905a0fef-bc53-11df-8c49-001e4fc686da")]
unsafe trait IBufferByteAccess: windows_core::IUnknown {
    fn Buffer(&self, value: *mut *mut u8) -> windows_core::HRESULT;
}

/// The Win32 window handle behind a WinUI window.
pub(crate) fn hwnd_of(window: &Window) -> windows_core::Result<isize> {
    let mut hwnd = 0isize;
    unsafe { window.cast::<IWindowNative>()?.WindowHandle(&mut hwnd).ok()? };
    Ok(hwnd)
}

/// A new bitmap holding `bgra` (premultiplied BGRA rows, `w * h * 4` bytes).
pub(crate) fn bitmap(w: usize, h: usize, bgra: &[u8]) -> windows_core::Result<WriteableBitmap> {
    let bmp = WriteableBitmap::CreateInstanceWithDimensions(w as i32, h as i32)?;
    let buf = bmp.PixelBuffer()?;
    let mut p: *mut u8 = std::ptr::null_mut();
    unsafe { buf.cast::<IBufferByteAccess>()?.Buffer(&mut p).ok()? };
    let len = (buf.Length()? as usize).min(bgra.len());
    unsafe { std::ptr::copy_nonoverlapping(bgra.as_ptr(), p, len) };
    bmp.Invalidate()?;
    Ok(bmp)
}

// ---------------------------------------------------------------- the XAML Application

#[implement(IApplicationOverrides, IXamlMetadataProvider)]
pub(crate) struct App {
    pub(crate) provider: OnceLock<XamlControlsXamlMetaDataProvider>,
}

impl App_Impl {
    fn provider(&self) -> windows_core::Result<&XamlControlsXamlMetaDataProvider> {
        if let Some(p) = self.provider.get() {
            return Ok(p);
        }
        let p = XamlControlsXamlMetaDataProvider::new()?;
        Ok(self.provider.get_or_init(|| p))
    }
}

impl IXamlMetadataProvider_Impl for App_Impl {
    fn GetXamlType(&self, t: &TypeName) -> windows_core::Result<IXamlType> {
        self.provider()?.GetXamlType(t)
    }
    fn GetXamlTypeByFullName(&self, name: &HSTRING) -> windows_core::Result<IXamlType> {
        self.provider()?.GetXamlTypeByFullName(name)
    }
    fn GetXmlnsDefinitions(&self) -> windows_core::Result<windows_core::Array<XmlnsDefinition>> {
        self.provider()?.GetXmlnsDefinitions()
    }
}

impl IApplicationOverrides_Impl for App_Impl {
    fn OnLaunched(&self, _args: Ref<LaunchActivatedEventArgs>) -> windows_core::Result<()> {
        if let Some(app) = APP.get() {
            let xcr = XamlControlsResources::new()?;
            app.Resources()?.MergedDictionaries()?.Append(&xcr.cast::<ResourceDictionary>()?)?;
            // Sprawl's own styles (look.rs), after the controls' so they can build on them
            let own: ResourceDictionary = crate::state::load(&crate::look::resources())?;
            app.Resources()?.MergedDictionaries()?.Append(&own)?;
        }
        if let Err(e) = crate::launch() {
            eprintln!("start-up failed: {e:?}");
            std::process::exit(2);
        }
        Ok(())
    }
}

/// Sprawl's own styles made again for the theme now in use, in place of the ones merged at
/// start (the last of the merged dictionaries; the controls' own stay). The tokens must have
/// been reset first, so the styles are made from the new theme's values.
pub(crate) fn refresh_resources() -> windows_core::Result<()> {
    let Some(app) = APP.get() else { return Ok(()) };
    let own: ResourceDictionary = crate::state::load(&crate::look::resources())?;
    let merged = app.Resources()?.MergedDictionaries()?;
    let n = merged.Size()?;
    if n >= 2 {
        merged.SetAt(n - 1, &own)?;
    } else {
        merged.Append(&own)?;
    }
    // WinUI refuses this once the app runs on some versions: the page's own RequestedTheme (on
    // its root) is what the controls follow, so a refusal costs nothing
    let _ = app.SetRequestedTheme(if crate::theme::light() { crate::bindings::Microsoft::UI::Xaml::ApplicationTheme::Light } else { crate::bindings::Microsoft::UI::Xaml::ApplicationTheme::Dark });
    Ok(())
}

pub(crate) static APP: OnceLock<Application> = OnceLock::new();
