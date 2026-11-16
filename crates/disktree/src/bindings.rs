pub mod Microsoft {
    pub mod UI {
        pub mod Dispatching {
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct DispatcherQueue(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                DispatcherQueue,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl DispatcherQueue {
                pub fn CreateTimer(&self) -> windows_core::Result<DispatcherQueueTimer> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).CreateTimer)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
                pub fn GetForCurrentThread() -> windows_core::Result<Self> {
                    Self::IDispatcherQueueStatics(|this| unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).GetForCurrentThread)(
                            windows_core::Interface::as_raw(this),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    })
                }
                fn IDispatcherQueueStatics<
                    R,
                    F: FnOnce(&IDispatcherQueueStatics) -> windows_core::Result<R>,
                >(
                    callback: F,
                ) -> windows_core::Result<R> {
                    static SHARED: windows_core::imp::FactoryCache<
                        DispatcherQueue,
                        IDispatcherQueueStatics,
                    > = windows_core::imp::FactoryCache::new();
                    SHARED.call(callback)
                }
            }
            impl windows_core::RuntimeType for DispatcherQueue {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IDispatcherQueue>();
            }
            unsafe impl windows_core::Interface for DispatcherQueue {
                type Vtable = <IDispatcherQueue as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID = <IDispatcherQueue as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for DispatcherQueue {
                const NAME: &'static str = "Microsoft.UI.Dispatching.DispatcherQueue";
            }
            unsafe impl Send for DispatcherQueue {}
            unsafe impl Sync for DispatcherQueue {}
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct DispatcherQueueTimer(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                DispatcherQueueTimer,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl DispatcherQueueTimer {
                pub fn Interval(&self) -> windows_core::Result<windows_time::TimeSpan> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Interval)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn SetInterval(
                    &self,
                    value: windows_time::TimeSpan,
                ) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetInterval)(
                            windows_core::Interface::as_raw(self),
                            value,
                        )
                        .ok()
                    }
                }
                pub fn Start(&self) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).Start)(
                            windows_core::Interface::as_raw(self),
                        )
                        .ok()
                    }
                }
                pub fn Stop(&self) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).Stop)(
                            windows_core::Interface::as_raw(self),
                        )
                        .ok()
                    }
                }
                pub fn Tick<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(windows_core::Ref<Self>, windows_core::Ref<windows_core::IInspectable>)
                        + Send
                        + 'static,
                {
                    let handler = <super::super::super::Windows::Foundation::TypedEventHandler<
                        Self,
                        windows_core::IInspectable,
                    >>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(self).Tick)(
                            windows_core::Interface::as_raw(self),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            self.clone(),
                            token__,
                            windows_core::Interface::vtable(self).RemoveTick,
                        ))
                    }
                }
            }
            impl windows_core::RuntimeType for DispatcherQueueTimer {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IDispatcherQueueTimer>();
            }
            unsafe impl windows_core::Interface for DispatcherQueueTimer {
                type Vtable = <IDispatcherQueueTimer as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID =
                    <IDispatcherQueueTimer as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for DispatcherQueueTimer {
                const NAME: &'static str = "Microsoft.UI.Dispatching.DispatcherQueueTimer";
            }
            unsafe impl Send for DispatcherQueueTimer {}
            unsafe impl Sync for DispatcherQueueTimer {}
            windows_core::imp::define_interface!(
                IDispatcherQueue,
                IDispatcherQueue_Vtbl,
                0xf6ebf8fa_be1c_5bf6_a467_73da28738ae8
            );
            impl windows_core::RuntimeType for IDispatcherQueue {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Dispatching.IDispatcherQueue",
                    );
            }
            #[repr(C)]
            pub struct IDispatcherQueue_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                pub CreateTimer: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                )
                    -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IDispatcherQueueStatics,
                IDispatcherQueueStatics_Vtbl,
                0xcd3382ea_a455_5124_b63a_ca40d34ca23c
            );
            impl windows_core::RuntimeType for IDispatcherQueueStatics {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Dispatching.IDispatcherQueueStatics",
                    );
            }
            #[repr(C)]
            pub struct IDispatcherQueueStatics_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                pub GetForCurrentThread: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                )
                    -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IDispatcherQueueTimer,
                IDispatcherQueueTimer_Vtbl,
                0xad4d63fd_88fe_541f_ac11_bf2dc1ed2ce5
            );
            impl windows_core::RuntimeType for IDispatcherQueueTimer {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Dispatching.IDispatcherQueueTimer",
                    );
            }
            #[repr(C)]
            pub struct IDispatcherQueueTimer_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                pub Interval: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut windows_time::TimeSpan,
                ) -> windows_core::HRESULT,
                pub SetInterval: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    windows_time::TimeSpan,
                )
                    -> windows_core::HRESULT,
                IsRunning: usize,
                IsRepeating: usize,
                SetIsRepeating: usize,
                pub Start:
                    unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
                pub Stop:
                    unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
                pub Tick: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                    *mut i64,
                ) -> windows_core::HRESULT,
                pub RemoveTick:
                    unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
            }
        }
        pub mod Input {
            windows_core::imp::define_interface!(
                IPointerPoint,
                IPointerPoint_Vtbl,
                0x0d430ee6_252c_59a4_b2a2_d44264dc6a40
            );
            impl windows_core::RuntimeType for IPointerPoint {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Microsoft.UI.Input.IPointerPoint");
            }
            #[repr(C)]
            pub struct IPointerPoint_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                FrameId: usize,
                IsInContact: usize,
                PointerDeviceType: usize,
                PointerId: usize,
                pub Position: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut super::super::super::Windows::Foundation::Point,
                ) -> windows_core::HRESULT,
            }
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct PointerPoint(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                PointerPoint,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl PointerPoint {
                pub fn Position(
                    &self,
                ) -> windows_core::Result<super::super::super::Windows::Foundation::Point>
                {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Position)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
            }
            impl windows_core::RuntimeType for PointerPoint {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IPointerPoint>();
            }
            unsafe impl windows_core::Interface for PointerPoint {
                type Vtable = <IPointerPoint as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID = <IPointerPoint as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for PointerPoint {
                const NAME: &'static str = "Microsoft.UI.Input.PointerPoint";
            }
            unsafe impl Send for PointerPoint {}
            unsafe impl Sync for PointerPoint {}
        }
        pub mod Windowing {
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct AppWindow(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                AppWindow,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl AppWindow {
                pub fn Resize(
                    &self,
                    size: super::super::super::Windows::Graphics::SizeInt32,
                ) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).Resize)(
                            windows_core::Interface::as_raw(self),
                            size,
                        )
                        .ok()
                    }
                }
                pub fn Show(&self) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).Show)(
                            windows_core::Interface::as_raw(self),
                        )
                        .ok()
                    }
                }


            }
            impl windows_core::RuntimeType for AppWindow {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IAppWindow>();
            }
            unsafe impl windows_core::Interface for AppWindow {
                type Vtable = <IAppWindow as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID = <IAppWindow as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for AppWindow {
                const NAME: &'static str = "Microsoft.UI.Windowing.AppWindow";
            }
            unsafe impl Send for AppWindow {}
            unsafe impl Sync for AppWindow {}
            windows_core::imp::define_interface!(
                IAppWindow,
                IAppWindow_Vtbl,
                0xcfa788b3_643b_5c5e_ad4e_321d48a82acd
            );
            impl windows_core::RuntimeType for IAppWindow {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Windowing.IAppWindow",
                    );
            }
            #[repr(C)]
            pub struct IAppWindow_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                Id: usize,
                IsShownInSwitchers: usize,
                SetIsShownInSwitchers: usize,
                IsVisible: usize,
                OwnerWindowId: usize,
                Position: usize,
                Presenter: usize,
                Size: usize,
                Title: usize,
                SetTitle: usize,
                TitleBar: usize,
                Destroy: usize,
                Hide: usize,
                Move: usize,
                MoveAndResize: usize,
                MoveAndResizeRelativeToDisplayArea: usize,
                pub Resize: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    super::super::super::Windows::Graphics::SizeInt32,
                ) -> windows_core::HRESULT,
                SetIcon: usize,
                SetIconWithIconId: usize,
                SetPresenter: usize,
                SetPresenterByKind: usize,
                pub Show:
                    unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
            }
        }
        pub mod Xaml {
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct Application(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                Application,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl Application {
                pub fn Resources(&self) -> windows_core::Result<ResourceDictionary> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Resources)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
                pub fn SetResources<P0>(&self, value: P0) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<ResourceDictionary>,
                {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetResources)(
                            windows_core::Interface::as_raw(self),
                            value.param().abi(),
                        )
                        .ok()
                    }
                }
                pub fn RequestedTheme(&self) -> windows_core::Result<ApplicationTheme> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).RequestedTheme)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn SetRequestedTheme(
                    &self,
                    value: ApplicationTheme,
                ) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetRequestedTheme)(
                            windows_core::Interface::as_raw(self),
                            value,
                        )
                        .ok()
                    }
                }
                pub fn Exit(&self) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).Exit)(
                            windows_core::Interface::as_raw(self),
                        )
                        .ok()
                    }
                }
                pub fn new() -> windows_core::Result<Self> {
                    Self::IApplicationFactory(|this| unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).CreateInstance)(
                            windows_core::Interface::as_raw(this),
                            core::ptr::null_mut(),
                            core::ptr::null_mut(),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    })
                }
                pub fn compose<T>(compose: T) -> windows_core::Result<Self>
                where
                    T: windows_core::Compose,
                {
                    Self::IApplicationFactory(|this| unsafe {
                        let (derived__, base__) = windows_core::Compose::compose(compose);
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).CreateInstance)(
                            windows_core::Interface::as_raw(this),
                            core::mem::transmute_copy(&derived__),
                            base__ as *mut _ as _,
                            &mut result__,
                        )
                        .ok()?;
                        let _ = &derived__;
                        windows_core::imp::Type::from_abi(result__)
                    })
                }
                pub fn OnLaunched<P0>(&self, args: P0) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<LaunchActivatedEventArgs>,
                {
                    let this = &windows_core::Interface::cast::<IApplicationOverrides>(self)?;
                    unsafe {
                        (windows_core::Interface::vtable(this).OnLaunched)(
                            windows_core::Interface::as_raw(this),
                            args.param().abi(),
                        )
                        .ok()
                    }
                }
                pub fn Start<P0>(callback: P0) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<ApplicationInitializationCallback>,
                {
                    Self::IApplicationStatics(|this| unsafe {
                        (windows_core::Interface::vtable(this).Start)(
                            windows_core::Interface::as_raw(this),
                            callback.param().abi(),
                        )
                        .ok()
                    })
                }
                fn IApplicationFactory<
                    R,
                    F: FnOnce(&IApplicationFactory) -> windows_core::Result<R>,
                >(
                    callback: F,
                ) -> windows_core::Result<R> {
                    static SHARED: windows_core::imp::FactoryCache<
                        Application,
                        IApplicationFactory,
                    > = windows_core::imp::FactoryCache::new();
                    SHARED.call(callback)
                }
                fn IApplicationStatics<
                    R,
                    F: FnOnce(&IApplicationStatics) -> windows_core::Result<R>,
                >(
                    callback: F,
                ) -> windows_core::Result<R> {
                    static SHARED: windows_core::imp::FactoryCache<
                        Application,
                        IApplicationStatics,
                    > = windows_core::imp::FactoryCache::new();
                    SHARED.call(callback)
                }
            }
            impl windows_core::RuntimeType for Application {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IApplication>();
            }
            unsafe impl windows_core::Interface for Application {
                type Vtable = <IApplication as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID = <IApplication as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for Application {
                const NAME: &'static str = "Microsoft.UI.Xaml.Application";
            }
            unsafe impl Send for Application {}
            unsafe impl Sync for Application {}
            windows_core::imp::define_interface!(
                ApplicationInitializationCallback,
                ApplicationInitializationCallback_Vtbl,
                0xd8eef1c9_1234_56f1_9963_45dd9c80a661
            );
            impl windows_core::RuntimeType for ApplicationInitializationCallback {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
            }
            impl ApplicationInitializationCallback {
                pub fn new<
                    F: Fn(
                            windows_core::Ref<ApplicationInitializationCallbackParams>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                >(
                    invoke: F,
                ) -> Self {
                    let com = windows_core::imp::DelegateBox::<Self, F>::new(
                        &ApplicationInitializationCallbackBox::<F>::VTABLE,
                        invoke,
                    );
                    unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
                }
                pub fn Invoke<P0>(&self, p: P0) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<ApplicationInitializationCallbackParams>,
                {
                    unsafe {
                        (windows_core::Interface::vtable(self).Invoke)(
                            windows_core::Interface::as_raw(self),
                            p.param().abi(),
                        )
                        .ok()
                    }
                }
            }
            #[repr(C)]
            pub struct ApplicationInitializationCallback_Vtbl {
                base__: windows_core::IUnknown_Vtbl,
                Invoke: unsafe extern "system" fn(
                    this: *mut core::ffi::c_void,
                    p: *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
            }
            struct ApplicationInitializationCallbackBox<
                F: Fn(
                        windows_core::Ref<ApplicationInitializationCallbackParams>,
                    ) -> windows_core::Result<()>
                    + Send
                    + 'static,
            >(core::marker::PhantomData<(fn() -> F,)>);
            impl<
                F: Fn(
                        windows_core::Ref<ApplicationInitializationCallbackParams>,
                    ) -> windows_core::Result<()>
                    + Send
                    + 'static,
            > ApplicationInitializationCallbackBox<F>
            {
                const VTABLE: ApplicationInitializationCallback_Vtbl =
                    ApplicationInitializationCallback_Vtbl {
                        base__: windows_core::IUnknown_Vtbl {
                            QueryInterface: windows_core::imp::DelegateBox::<
                                ApplicationInitializationCallback,
                                F,
                            >::QueryInterface,
                            AddRef: windows_core::imp::DelegateBox::<
                                ApplicationInitializationCallback,
                                F,
                            >::AddRef,
                            Release: windows_core::imp::DelegateBox::<
                                ApplicationInitializationCallback,
                                F,
                            >::Release,
                        },
                        Invoke: Self::Invoke,
                    };
                unsafe extern "system" fn Invoke(
                    this: *mut core::ffi::c_void,
                    p: *mut core::ffi::c_void,
                ) -> windows_core::HRESULT {
                    unsafe {
                        let this = &mut *(this as *mut *mut core::ffi::c_void
                            as *mut windows_core::imp::DelegateBox<
                                ApplicationInitializationCallback,
                                F,
                            >);
                        (this.invoke)(core::mem::transmute_copy(&p)).into()
                    }
                }
            }
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct ApplicationInitializationCallbackParams(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                ApplicationInitializationCallbackParams,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl windows_core::RuntimeType for ApplicationInitializationCallbackParams {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<
                        Self,
                        IApplicationInitializationCallbackParams,
                    >();
            }
            unsafe impl windows_core::Interface for ApplicationInitializationCallbackParams {
                type Vtable =
                    <IApplicationInitializationCallbackParams as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID =
                    <IApplicationInitializationCallbackParams as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for ApplicationInitializationCallbackParams {
                const NAME: &'static str =
                    "Microsoft.UI.Xaml.ApplicationInitializationCallbackParams";
            }
            unsafe impl Send for ApplicationInitializationCallbackParams {}
            unsafe impl Sync for ApplicationInitializationCallbackParams {}
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
            pub struct ApplicationTheme(pub i32);
            impl ApplicationTheme {
                pub const Light: Self = Self(0);
                pub const Dark: Self = Self(1);
            }
            impl windows_core::imp::TypeKind for ApplicationTheme {
                type TypeKind = windows_core::imp::CopyType;
            }
            impl windows_core::RuntimeType for ApplicationTheme {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"enum(Microsoft.UI.Xaml.ApplicationTheme;i4)",
                    );
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.ApplicationTheme",
                    );
            }
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct DependencyObject(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                DependencyObject,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl DependencyObject {

            }
            impl windows_core::RuntimeType for DependencyObject {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IDependencyObject>();
            }
            unsafe impl windows_core::Interface for DependencyObject {
                type Vtable = <IDependencyObject as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID = <IDependencyObject as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for DependencyObject {
                const NAME: &'static str = "Microsoft.UI.Xaml.DependencyObject";
            }
            unsafe impl Send for DependencyObject {}
            unsafe impl Sync for DependencyObject {}
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct FrameworkElement(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                FrameworkElement,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            windows_core::imp::required_hierarchy!(FrameworkElement, UIElement, DependencyObject);
            impl FrameworkElement {
                pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).ActualWidth)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).ActualHeight)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn Width(&self) -> windows_core::Result<f64> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Width)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetWidth)(
                            windows_core::Interface::as_raw(self),
                            value,
                        )
                        .ok()
                    }
                }
                pub fn SizeChanged<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<SizeChangedEventArgs>,
                        ) + Send
                        + 'static,
                {
                    let handler = <SizeChangedEventHandler>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(self).SizeChanged)(
                            windows_core::Interface::as_raw(self),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            self.clone(),
                            token__,
                            windows_core::Interface::vtable(self).RemoveSizeChanged,
                        ))
                    }
                }
                pub fn FindName(
                    &self,
                    name: &windows_core::HSTRING,
                ) -> windows_core::Result<windows_core::IInspectable> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).FindName)(
                            windows_core::Interface::as_raw(self),
                            core::mem::transmute_copy(name),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
                pub fn Visibility(&self) -> windows_core::Result<Visibility> {
                    let this = &windows_core::Interface::cast::<IUIElement>(self)?;
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).Visibility)(
                            windows_core::Interface::as_raw(this),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn SetVisibility(&self, value: Visibility) -> windows_core::Result<()> {
                    let this = &windows_core::Interface::cast::<IUIElement>(self)?;
                    unsafe {
                        (windows_core::Interface::vtable(this).SetVisibility)(
                            windows_core::Interface::as_raw(this),
                            value,
                        )
                        .ok()
                    }
                }
                pub fn PointerMoved<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<Input::PointerRoutedEventArgs>,
                        ) + Send
                        + 'static,
                {
                    let this = &windows_core::Interface::cast::<IUIElement>(self)?;
                    let handler = <Input::PointerEventHandler>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                            windows_core::Interface::as_raw(this),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            this.clone(),
                            token__,
                            windows_core::Interface::vtable(this).RemovePointerMoved,
                        ))
                    }
                }
                pub fn PointerExited<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<Input::PointerRoutedEventArgs>,
                        ) + Send
                        + 'static,
                {
                    let this = &windows_core::Interface::cast::<IUIElement>(self)?;
                    let handler = <Input::PointerEventHandler>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                            windows_core::Interface::as_raw(this),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            this.clone(),
                            token__,
                            windows_core::Interface::vtable(this).RemovePointerExited,
                        ))
                    }
                }
                pub fn Tapped<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<Input::TappedRoutedEventArgs>,
                        ) + Send
                        + 'static,
                {
                    let this = &windows_core::Interface::cast::<IUIElement>(self)?;
                    let handler = <Input::TappedEventHandler>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(this).Tapped)(
                            windows_core::Interface::as_raw(this),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            this.clone(),
                            token__,
                            windows_core::Interface::vtable(this).RemoveTapped,
                        ))
                    }
                }
                pub fn DoubleTapped<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<Input::DoubleTappedRoutedEventArgs>,
                        ) + Send
                        + 'static,
                {
                    let this = &windows_core::Interface::cast::<IUIElement>(self)?;
                    let handler = <Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                            windows_core::Interface::as_raw(this),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            this.clone(),
                            token__,
                            windows_core::Interface::vtable(this).RemoveDoubleTapped,
                        ))
                    }
                }


            }
            impl windows_core::RuntimeType for FrameworkElement {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IFrameworkElement>();
            }
            unsafe impl windows_core::Interface for FrameworkElement {
                type Vtable = <IFrameworkElement as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID = <IFrameworkElement as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for FrameworkElement {
                const NAME: &'static str = "Microsoft.UI.Xaml.FrameworkElement";
            }
            unsafe impl Send for FrameworkElement {}
            unsafe impl Sync for FrameworkElement {}
            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default, PartialEq)]
            pub struct GridLength {
                pub Value: f64,
                pub GridUnitType: GridUnitType,
            }
            impl windows_core::imp::TypeKind for GridLength {
                type TypeKind = windows_core::imp::CopyType;
            }
            impl windows_core::RuntimeType for GridLength {
                const SIGNATURE : windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice (b"struct(Microsoft.UI.Xaml.GridLength;f8;enum(Microsoft.UI.Xaml.GridUnitType;i4))") ;
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Microsoft.UI.Xaml.GridLength");
            }
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
            pub struct GridUnitType(pub i32);
            impl GridUnitType {
                pub const Auto: Self = Self(0);
                pub const Pixel: Self = Self(1);
                pub const Star: Self = Self(2);
            }
            impl windows_core::imp::TypeKind for GridUnitType {
                type TypeKind = windows_core::imp::CopyType;
            }
            impl windows_core::RuntimeType for GridUnitType {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"enum(Microsoft.UI.Xaml.GridUnitType;i4)",
                    );
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Microsoft.UI.Xaml.GridUnitType");
            }
            windows_core::imp::define_interface!(
                IApplication,
                IApplication_Vtbl,
                0x06a8f4e7_1146_55af_820d_ebd55643b021
            );
            impl windows_core::RuntimeType for IApplication {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Microsoft.UI.Xaml.IApplication");
            }
            #[repr(C)]
            pub struct IApplication_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                pub Resources: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
                pub SetResources: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                )
                    -> windows_core::HRESULT,
                DebugSettings: usize,
                pub RequestedTheme: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut ApplicationTheme,
                )
                    -> windows_core::HRESULT,
                pub SetRequestedTheme: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    ApplicationTheme,
                )
                    -> windows_core::HRESULT,
                FocusVisualKind: usize,
                SetFocusVisualKind: usize,
                HighContrastAdjustment: usize,
                SetHighContrastAdjustment: usize,
                UnhandledException: usize,
                RemoveUnhandledException: usize,
                pub Exit:
                    unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IApplicationFactory,
                IApplicationFactory_Vtbl,
                0x9fd96657_5294_5a65_a1db_4fea143597da
            );
            impl windows_core::RuntimeType for IApplicationFactory {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.IApplicationFactory",
                    );
            }
            #[repr(C)]
            pub struct IApplicationFactory_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                pub CreateInstance: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                )
                    -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IApplicationInitializationCallbackParams,
                IApplicationInitializationCallbackParams_Vtbl,
                0x1b1906ea_5b7b_5876_81ab_7c2281ac3d20
            );
            impl windows_core::RuntimeType for IApplicationInitializationCallbackParams {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.IApplicationInitializationCallbackParams",
                    );
            }
            #[repr(C)]
            pub struct IApplicationInitializationCallbackParams_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
            }
            windows_core::imp::define_interface!(
                IApplicationOverrides,
                IApplicationOverrides_Vtbl,
                0xa33e81ef_c665_503b_8827_d27ef1720a06
            );
            impl windows_core::RuntimeType for IApplicationOverrides {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.IApplicationOverrides",
                    );
            }
            impl windows_core::RuntimeName for IApplicationOverrides {
                const NAME: &'static str = "Microsoft.UI.Xaml.IApplicationOverrides";
            }
            pub trait IApplicationOverrides_Impl: windows_core::IUnknownImpl {
                fn OnLaunched(
                    &self,
                    args: windows_core::Ref<LaunchActivatedEventArgs>,
                ) -> windows_core::Result<()>;
            }
            impl IApplicationOverrides_Vtbl {
                pub const fn new<Identity: IApplicationOverrides_Impl, const OFFSET: isize>() -> Self
                {
                    unsafe extern "system" fn OnLaunched<
                        Identity: IApplicationOverrides_Impl,
                        const OFFSET: isize,
                    >(
                        this: *mut core::ffi::c_void,
                        args: *mut core::ffi::c_void,
                    ) -> windows_core::HRESULT {
                        unsafe {
                            let this: &Identity =
                                &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                            IApplicationOverrides_Impl::OnLaunched(
                                this,
                                core::mem::transmute_copy(&args),
                            )
                            .into()
                        }
                    }
                    Self {
                        base__: windows_core::IInspectable_Vtbl::new::<
                            Identity,
                            IApplicationOverrides,
                            OFFSET,
                        >(),
                        OnLaunched: OnLaunched::<Identity, OFFSET>,
                    }
                }
                pub fn matches(iid: &windows_core::GUID) -> bool {
                    iid == &<IApplicationOverrides as windows_core::Interface>::IID
                }
            }
            #[repr(C)]
            pub struct IApplicationOverrides_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                pub OnLaunched: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IApplicationStatics,
                IApplicationStatics_Vtbl,
                0x4e0d09f5_4358_512c_a987_503b52848e95
            );
            impl windows_core::RuntimeType for IApplicationStatics {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.IApplicationStatics",
                    );
            }
            #[repr(C)]
            pub struct IApplicationStatics_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                Current: usize,
                pub Start: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IDependencyObject,
                IDependencyObject_Vtbl,
                0xe7beaee7_160e_50f7_8789_d63463f979fa
            );
            impl windows_core::RuntimeType for IDependencyObject {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.IDependencyObject",
                    );
            }
            #[repr(C)]
            pub struct IDependencyObject_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
            }
            windows_core::imp::define_interface!(
                IFrameworkElement,
                IFrameworkElement_Vtbl,
                0xfe08f13d_dc6a_5495_ad44_c2d8d21863b0
            );
            impl windows_core::RuntimeType for IFrameworkElement {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.IFrameworkElement",
                    );
            }
            #[repr(C)]
            pub struct IFrameworkElement_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                Triggers: usize,
                Resources: usize,
                SetResources: usize,
                Tag: usize,
                SetTag: usize,
                Language: usize,
                SetLanguage: usize,
                pub ActualWidth: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut f64,
                )
                    -> windows_core::HRESULT,
                pub ActualHeight: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut f64,
                )
                    -> windows_core::HRESULT,
                pub Width: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut f64,
                ) -> windows_core::HRESULT,
                pub SetWidth:
                    unsafe extern "system" fn(*mut core::ffi::c_void, f64) -> windows_core::HRESULT,
                Height: usize,
                SetHeight: usize,
                MinWidth: usize,
                SetMinWidth: usize,
                MaxWidth: usize,
                SetMaxWidth: usize,
                MinHeight: usize,
                SetMinHeight: usize,
                MaxHeight: usize,
                SetMaxHeight: usize,
                HorizontalAlignment: usize,
                SetHorizontalAlignment: usize,
                VerticalAlignment: usize,
                SetVerticalAlignment: usize,
                Margin: usize,
                SetMargin: usize,
                Name: usize,
                SetName: usize,
                BaseUri: usize,
                DataContext: usize,
                SetDataContext: usize,
                AllowFocusOnInteraction: usize,
                SetAllowFocusOnInteraction: usize,
                FocusVisualMargin: usize,
                SetFocusVisualMargin: usize,
                FocusVisualSecondaryThickness: usize,
                SetFocusVisualSecondaryThickness: usize,
                FocusVisualPrimaryThickness: usize,
                SetFocusVisualPrimaryThickness: usize,
                FocusVisualSecondaryBrush: usize,
                SetFocusVisualSecondaryBrush: usize,
                FocusVisualPrimaryBrush: usize,
                SetFocusVisualPrimaryBrush: usize,
                AllowFocusWhenDisabled: usize,
                SetAllowFocusWhenDisabled: usize,
                Style: usize,
                SetStyle: usize,
                Parent: usize,
                FlowDirection: usize,
                SetFlowDirection: usize,
                RequestedTheme: usize,
                SetRequestedTheme: usize,
                IsLoaded: usize,
                ActualTheme: usize,
                Loaded: usize,
                RemoveLoaded: usize,
                Unloaded: usize,
                RemoveUnloaded: usize,
                DataContextChanged: usize,
                RemoveDataContextChanged: usize,
                pub SizeChanged: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                    *mut i64,
                )
                    -> windows_core::HRESULT,
                pub RemoveSizeChanged:
                    unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
                LayoutUpdated: usize,
                RemoveLayoutUpdated: usize,
                Loading: usize,
                RemoveLoading: usize,
                ActualThemeChanged: usize,
                RemoveActualThemeChanged: usize,
                EffectiveViewportChanged: usize,
                RemoveEffectiveViewportChanged: usize,
                pub FindName: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                ILaunchActivatedEventArgs,
                ILaunchActivatedEventArgs_Vtbl,
                0xd505cea9_1bcb_5b29_a8be_944e00f06f78
            );
            impl windows_core::RuntimeType for ILaunchActivatedEventArgs {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.ILaunchActivatedEventArgs",
                    );
            }
            #[repr(C)]
            pub struct ILaunchActivatedEventArgs_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
            }
            windows_core::imp::define_interface!(
                IResourceDictionary,
                IResourceDictionary_Vtbl,
                0x1b690975_a710_5783_a6e1_15836f6186c2
            );
            impl windows_core::RuntimeType for IResourceDictionary {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.IResourceDictionary",
                    );
            }
            #[repr(C)]
            pub struct IResourceDictionary_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                Source: usize,
                SetSource: usize,
                pub MergedDictionaries: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                )
                    -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IRoutedEventArgs,
                IRoutedEventArgs_Vtbl,
                0x0908c407_1c7d_5de3_9c50_d971c62ec8ec
            );
            impl windows_core::RuntimeType for IRoutedEventArgs {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.IRoutedEventArgs",
                    );
            }
            #[repr(C)]
            pub struct IRoutedEventArgs_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
            }
            windows_core::imp::define_interface!(
                ISizeChangedEventArgs,
                ISizeChangedEventArgs_Vtbl,
                0xfe76324e_6dfb_58b1_9dcd_886ca8f9a2ea
            );
            impl windows_core::RuntimeType for ISizeChangedEventArgs {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"Microsoft.UI.Xaml.ISizeChangedEventArgs",
                    );
            }
            #[repr(C)]
            pub struct ISizeChangedEventArgs_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
            }
            windows_core::imp::define_interface!(
                IUIElement,
                IUIElement_Vtbl,
                0xc3c01020_320c_5cf6_9d24_d396bbfa4d8b
            );
            impl windows_core::RuntimeType for IUIElement {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Microsoft.UI.Xaml.IUIElement");
            }
            #[repr(C)]
            pub struct IUIElement_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                DesiredSize: usize,
                AllowDrop: usize,
                SetAllowDrop: usize,
                Opacity: usize,
                SetOpacity: usize,
                Clip: usize,
                SetClip: usize,
                RenderTransform: usize,
                SetRenderTransform: usize,
                Projection: usize,
                SetProjection: usize,
                Transform3D: usize,
                SetTransform3D: usize,
                RenderTransformOrigin: usize,
                SetRenderTransformOrigin: usize,
                IsHitTestVisible: usize,
                SetIsHitTestVisible: usize,
                pub Visibility: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut Visibility,
                ) -> windows_core::HRESULT,
                pub SetVisibility: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    Visibility,
                )
                    -> windows_core::HRESULT,
                RenderSize: usize,
                UseLayoutRounding: usize,
                SetUseLayoutRounding: usize,
                Transitions: usize,
                SetTransitions: usize,
                CacheMode: usize,
                SetCacheMode: usize,
                IsTapEnabled: usize,
                SetIsTapEnabled: usize,
                IsDoubleTapEnabled: usize,
                SetIsDoubleTapEnabled: usize,
                CanDrag: usize,
                SetCanDrag: usize,
                IsRightTapEnabled: usize,
                SetIsRightTapEnabled: usize,
                IsHoldingEnabled: usize,
                SetIsHoldingEnabled: usize,
                ManipulationMode: usize,
                SetManipulationMode: usize,
                PointerCaptures: usize,
                ContextFlyout: usize,
                SetContextFlyout: usize,
                CompositeMode: usize,
                SetCompositeMode: usize,
                Lights: usize,
                CanBeScrollAnchor: usize,
                SetCanBeScrollAnchor: usize,
                ExitDisplayModeOnAccessKeyInvoked: usize,
                SetExitDisplayModeOnAccessKeyInvoked: usize,
                IsAccessKeyScope: usize,
                SetIsAccessKeyScope: usize,
                AccessKeyScopeOwner: usize,
                SetAccessKeyScopeOwner: usize,
                AccessKey: usize,
                SetAccessKey: usize,
                KeyTipPlacementMode: usize,
                SetKeyTipPlacementMode: usize,
                KeyTipHorizontalOffset: usize,
                SetKeyTipHorizontalOffset: usize,
                KeyTipVerticalOffset: usize,
                SetKeyTipVerticalOffset: usize,
                KeyTipTarget: usize,
                SetKeyTipTarget: usize,
                XYFocusKeyboardNavigation: usize,
                SetXYFocusKeyboardNavigation: usize,
                XYFocusUpNavigationStrategy: usize,
                SetXYFocusUpNavigationStrategy: usize,
                XYFocusDownNavigationStrategy: usize,
                SetXYFocusDownNavigationStrategy: usize,
                XYFocusLeftNavigationStrategy: usize,
                SetXYFocusLeftNavigationStrategy: usize,
                XYFocusRightNavigationStrategy: usize,
                SetXYFocusRightNavigationStrategy: usize,
                KeyboardAccelerators: usize,
                KeyboardAcceleratorPlacementTarget: usize,
                SetKeyboardAcceleratorPlacementTarget: usize,
                KeyboardAcceleratorPlacementMode: usize,
                SetKeyboardAcceleratorPlacementMode: usize,
                HighContrastAdjustment: usize,
                SetHighContrastAdjustment: usize,
                TabFocusNavigation: usize,
                SetTabFocusNavigation: usize,
                OpacityTransition: usize,
                SetOpacityTransition: usize,
                Translation: usize,
                SetTranslation: usize,
                TranslationTransition: usize,
                SetTranslationTransition: usize,
                Rotation: usize,
                SetRotation: usize,
                RotationTransition: usize,
                SetRotationTransition: usize,
                Scale: usize,
                SetScale: usize,
                ScaleTransition: usize,
                SetScaleTransition: usize,
                TransformMatrix: usize,
                SetTransformMatrix: usize,
                CenterPoint: usize,
                SetCenterPoint: usize,
                RotationAxis: usize,
                SetRotationAxis: usize,
                ActualOffset: usize,
                ActualSize: usize,
                XamlRoot: usize,
                SetXamlRoot: usize,
                Shadow: usize,
                SetShadow: usize,
                RasterizationScale: usize,
                SetRasterizationScale: usize,
                FocusState: usize,
                UseSystemFocusVisuals: usize,
                SetUseSystemFocusVisuals: usize,
                XYFocusLeft: usize,
                SetXYFocusLeft: usize,
                XYFocusRight: usize,
                SetXYFocusRight: usize,
                XYFocusUp: usize,
                SetXYFocusUp: usize,
                XYFocusDown: usize,
                SetXYFocusDown: usize,
                IsTabStop: usize,
                SetIsTabStop: usize,
                TabIndex: usize,
                SetTabIndex: usize,
                KeyUp: usize,
                RemoveKeyUp: usize,
                KeyDown: usize,
                RemoveKeyDown: usize,
                GotFocus: usize,
                RemoveGotFocus: usize,
                LostFocus: usize,
                RemoveLostFocus: usize,
                DragStarting: usize,
                RemoveDragStarting: usize,
                DropCompleted: usize,
                RemoveDropCompleted: usize,
                CharacterReceived: usize,
                RemoveCharacterReceived: usize,
                DragEnter: usize,
                RemoveDragEnter: usize,
                DragLeave: usize,
                RemoveDragLeave: usize,
                DragOver: usize,
                RemoveDragOver: usize,
                Drop: usize,
                RemoveDrop: usize,
                PointerPressed: usize,
                RemovePointerPressed: usize,
                pub PointerMoved: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                    *mut i64,
                )
                    -> windows_core::HRESULT,
                pub RemovePointerMoved:
                    unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
                PointerReleased: usize,
                RemovePointerReleased: usize,
                PointerEntered: usize,
                RemovePointerEntered: usize,
                pub PointerExited: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                    *mut i64,
                )
                    -> windows_core::HRESULT,
                pub RemovePointerExited:
                    unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
                PointerCaptureLost: usize,
                RemovePointerCaptureLost: usize,
                PointerCanceled: usize,
                RemovePointerCanceled: usize,
                PointerWheelChanged: usize,
                RemovePointerWheelChanged: usize,
                pub Tapped: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                    *mut i64,
                ) -> windows_core::HRESULT,
                pub RemoveTapped:
                    unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
                pub DoubleTapped: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                    *mut i64,
                )
                    -> windows_core::HRESULT,
                pub RemoveDoubleTapped:
                    unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IWindow,
                IWindow_Vtbl,
                0x61f0ec79_5d52_56b5_86fb_40fa4af288b0
            );
            impl windows_core::RuntimeType for IWindow {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Microsoft.UI.Xaml.IWindow");
            }
            #[repr(C)]
            pub struct IWindow_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                Bounds: usize,
                Visible: usize,
                pub Content: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
                pub SetContent: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
                CoreWindow: usize,
                Compositor: usize,
                Dispatcher: usize,
                DispatcherQueue: usize,
                pub Title: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
                pub SetTitle: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
                pub ExtendsContentIntoTitleBar: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut bool,
                )
                    -> windows_core::HRESULT,
                pub SetExtendsContentIntoTitleBar:
                    unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        bool,
                    ) -> windows_core::HRESULT,
                Activated: usize,
                RemoveActivated: usize,
                Closed: usize,
                RemoveClosed: usize,
                SizeChanged: usize,
                RemoveSizeChanged: usize,
                VisibilityChanged: usize,
                RemoveVisibilityChanged: usize,
                pub Activate:
                    unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
                pub Close:
                    unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
                pub SetTitleBar: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                )
                    -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IWindow2,
                IWindow2_Vtbl,
                0x42febaa5_1c32_522a_a591_57618c6f665d
            );
            impl windows_core::RuntimeType for IWindow2 {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Microsoft.UI.Xaml.IWindow2");
            }
            #[repr(C)]
            pub struct IWindow2_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                pub SystemBackdrop: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                )
                    -> windows_core::HRESULT,
                pub SetSystemBackdrop: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                )
                    -> windows_core::HRESULT,
                pub AppWindow: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
            }
            windows_core::imp::define_interface!(
                IWindowFactory,
                IWindowFactory_Vtbl,
                0xf0441536_afef_5222_918f_324a9b2dec75
            );
            impl windows_core::RuntimeType for IWindowFactory {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Microsoft.UI.Xaml.IWindowFactory");
            }
            #[repr(C)]
            pub struct IWindowFactory_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                pub CreateInstance: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                )
                    -> windows_core::HRESULT,
            }
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct LaunchActivatedEventArgs(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                LaunchActivatedEventArgs,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl windows_core::RuntimeType for LaunchActivatedEventArgs {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, ILaunchActivatedEventArgs>();
            }
            unsafe impl windows_core::Interface for LaunchActivatedEventArgs {
                type Vtable = <ILaunchActivatedEventArgs as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID =
                    <ILaunchActivatedEventArgs as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for LaunchActivatedEventArgs {
                const NAME: &'static str = "Microsoft.UI.Xaml.LaunchActivatedEventArgs";
            }
            unsafe impl Send for LaunchActivatedEventArgs {}
            unsafe impl Sync for LaunchActivatedEventArgs {}
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct ResourceDictionary(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                ResourceDictionary,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            windows_core::imp::required_hierarchy!(
                ResourceDictionary,
                super::super::super::Windows::Foundation::Collections::IIterable<
                    windows_collections::IKeyValuePair<
                        windows_core::IInspectable,
                        windows_core::IInspectable,
                    >,
                >,
                DependencyObject
            );
            impl ResourceDictionary {
                pub fn First(
                    &self,
                ) -> windows_core::Result<
                    windows_collections::IIterator<
                        windows_collections::IKeyValuePair<
                            windows_core::IInspectable,
                            windows_core::IInspectable,
                        >,
                    >,
                > {
                    let this = &windows_core::Interface::cast::<
                        super::super::super::Windows::Foundation::Collections::IIterable<
                            windows_collections::IKeyValuePair<
                                windows_core::IInspectable,
                                windows_core::IInspectable,
                            >,
                        >,
                    >(self)?;
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).First)(
                            windows_core::Interface::as_raw(this),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
                pub fn MergedDictionaries(
                    &self,
                ) -> windows_core::Result<
                    super::super::super::Windows::Foundation::Collections::IVector<Self>,
                > {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).MergedDictionaries)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }

            }
            impl windows_core::RuntimeType for ResourceDictionary {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IResourceDictionary>();
            }
            unsafe impl windows_core::Interface for ResourceDictionary {
                type Vtable = <IResourceDictionary as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID =
                    <IResourceDictionary as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for ResourceDictionary {
                const NAME: &'static str = "Microsoft.UI.Xaml.ResourceDictionary";
            }
            unsafe impl Send for ResourceDictionary {}
            unsafe impl Sync for ResourceDictionary {}
            impl IntoIterator for ResourceDictionary {
                type Item = windows_collections::IKeyValuePair<
                    windows_core::IInspectable,
                    windows_core::IInspectable,
                >;
                type IntoIter = windows_collections::BufferedIterator<Self::Item>;
                fn into_iter(self) -> Self::IntoIter {
                    IntoIterator::into_iter(&self)
                }
            }
            impl IntoIterator for &ResourceDictionary {
                type Item = windows_collections::IKeyValuePair<
                    windows_core::IInspectable,
                    windows_core::IInspectable,
                >;
                type IntoIter = windows_collections::BufferedIterator<Self::Item>;
                fn into_iter(self) -> Self::IntoIter {
                    windows_collections::BufferedIterator::new(self.First().unwrap())
                }
            }
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct RoutedEventArgs(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                RoutedEventArgs,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl RoutedEventArgs {

            }
            impl windows_core::RuntimeType for RoutedEventArgs {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IRoutedEventArgs>();
            }
            unsafe impl windows_core::Interface for RoutedEventArgs {
                type Vtable = <IRoutedEventArgs as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID = <IRoutedEventArgs as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for RoutedEventArgs {
                const NAME: &'static str = "Microsoft.UI.Xaml.RoutedEventArgs";
            }
            unsafe impl Send for RoutedEventArgs {}
            unsafe impl Sync for RoutedEventArgs {}
            windows_core::imp::define_interface!(
                RoutedEventHandler,
                RoutedEventHandler_Vtbl,
                0xdae23d85_69ca_5bdf_805b_6161a3a215cc
            );
            impl windows_core::RuntimeType for RoutedEventHandler {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
            }
            impl RoutedEventHandler {
                pub fn new<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<RoutedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                >(
                    invoke: F,
                ) -> Self {
                    let com = windows_core::imp::DelegateBox::<Self, F>::new(
                        &RoutedEventHandlerBox::<F>::VTABLE,
                        invoke,
                    );
                    unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
                }
                pub fn Invoke<P0, P1>(&self, sender: P0, e: P1) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<windows_core::IInspectable>,
                    P1: windows_core::Param<RoutedEventArgs>,
                {
                    unsafe {
                        (windows_core::Interface::vtable(self).Invoke)(
                            windows_core::Interface::as_raw(self),
                            sender.param().abi(),
                            e.param().abi(),
                        )
                        .ok()
                    }
                }
            }
            #[repr(C)]
            pub struct RoutedEventHandler_Vtbl {
                base__: windows_core::IUnknown_Vtbl,
                Invoke: unsafe extern "system" fn(
                    this: *mut core::ffi::c_void,
                    sender: *mut core::ffi::c_void,
                    e: *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
            }
            struct RoutedEventHandlerBox<
                F: Fn(
                        windows_core::Ref<windows_core::IInspectable>,
                        windows_core::Ref<RoutedEventArgs>,
                    ) -> windows_core::Result<()>
                    + Send
                    + 'static,
            >(core::marker::PhantomData<(fn() -> F,)>);
            impl<
                F: Fn(
                        windows_core::Ref<windows_core::IInspectable>,
                        windows_core::Ref<RoutedEventArgs>,
                    ) -> windows_core::Result<()>
                    + Send
                    + 'static,
            > RoutedEventHandlerBox<F>
            {
                const VTABLE: RoutedEventHandler_Vtbl = RoutedEventHandler_Vtbl {
                    base__: windows_core::IUnknown_Vtbl {
                        QueryInterface:
                            windows_core::imp::DelegateBox::<RoutedEventHandler, F>::QueryInterface,
                        AddRef: windows_core::imp::DelegateBox::<RoutedEventHandler, F>::AddRef,
                        Release: windows_core::imp::DelegateBox::<RoutedEventHandler, F>::Release,
                    },
                    Invoke: Self::Invoke,
                };
                unsafe extern "system" fn Invoke(
                    this: *mut core::ffi::c_void,
                    sender: *mut core::ffi::c_void,
                    e: *mut core::ffi::c_void,
                ) -> windows_core::HRESULT {
                    unsafe {
                        let this = &mut *(this as *mut *mut core::ffi::c_void
                            as *mut windows_core::imp::DelegateBox<RoutedEventHandler, F>);
                        (this.invoke)(
                            core::mem::transmute_copy(&sender),
                            core::mem::transmute_copy(&e),
                        )
                        .into()
                    }
                }
            }
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct SizeChangedEventArgs(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                SizeChangedEventArgs,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            windows_core::imp::required_hierarchy!(SizeChangedEventArgs, RoutedEventArgs);
            impl windows_core::RuntimeType for SizeChangedEventArgs {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, ISizeChangedEventArgs>();
            }
            unsafe impl windows_core::Interface for SizeChangedEventArgs {
                type Vtable = <ISizeChangedEventArgs as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID =
                    <ISizeChangedEventArgs as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for SizeChangedEventArgs {
                const NAME: &'static str = "Microsoft.UI.Xaml.SizeChangedEventArgs";
            }
            unsafe impl Send for SizeChangedEventArgs {}
            unsafe impl Sync for SizeChangedEventArgs {}
            windows_core::imp::define_interface!(
                SizeChangedEventHandler,
                SizeChangedEventHandler_Vtbl,
                0x8d7b1a58_14c6_51c9_892c_9fcce368e77d
            );
            impl windows_core::RuntimeType for SizeChangedEventHandler {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
            }
            impl SizeChangedEventHandler {
                pub fn new<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<SizeChangedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                >(
                    invoke: F,
                ) -> Self {
                    let com = windows_core::imp::DelegateBox::<Self, F>::new(
                        &SizeChangedEventHandlerBox::<F>::VTABLE,
                        invoke,
                    );
                    unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
                }
                pub fn Invoke<P0, P1>(&self, sender: P0, e: P1) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<windows_core::IInspectable>,
                    P1: windows_core::Param<SizeChangedEventArgs>,
                {
                    unsafe {
                        (windows_core::Interface::vtable(self).Invoke)(
                            windows_core::Interface::as_raw(self),
                            sender.param().abi(),
                            e.param().abi(),
                        )
                        .ok()
                    }
                }
            }
            #[repr(C)]
            pub struct SizeChangedEventHandler_Vtbl {
                base__: windows_core::IUnknown_Vtbl,
                Invoke: unsafe extern "system" fn(
                    this: *mut core::ffi::c_void,
                    sender: *mut core::ffi::c_void,
                    e: *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
            }
            struct SizeChangedEventHandlerBox<
                F: Fn(
                        windows_core::Ref<windows_core::IInspectable>,
                        windows_core::Ref<SizeChangedEventArgs>,
                    ) -> windows_core::Result<()>
                    + Send
                    + 'static,
            >(core::marker::PhantomData<(fn() -> F,)>);
            impl<
                F: Fn(
                        windows_core::Ref<windows_core::IInspectable>,
                        windows_core::Ref<SizeChangedEventArgs>,
                    ) -> windows_core::Result<()>
                    + Send
                    + 'static,
            > SizeChangedEventHandlerBox<F>
            {
                const VTABLE : SizeChangedEventHandler_Vtbl < > = SizeChangedEventHandler_Vtbl::< > { base__ : windows_core::IUnknown_Vtbl { QueryInterface : windows_core::imp::DelegateBox::< SizeChangedEventHandler , F >::QueryInterface , AddRef : windows_core::imp::DelegateBox::< SizeChangedEventHandler , F >::AddRef , Release : windows_core::imp::DelegateBox::< SizeChangedEventHandler , F >::Release , } , Invoke : Self::Invoke , } ;
                unsafe extern "system" fn Invoke(
                    this: *mut core::ffi::c_void,
                    sender: *mut core::ffi::c_void,
                    e: *mut core::ffi::c_void,
                ) -> windows_core::HRESULT {
                    unsafe {
                        let this = &mut *(this as *mut *mut core::ffi::c_void
                            as *mut windows_core::imp::DelegateBox<SizeChangedEventHandler, F>);
                        (this.invoke)(
                            core::mem::transmute_copy(&sender),
                            core::mem::transmute_copy(&e),
                        )
                        .into()
                    }
                }
            }
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct UIElement(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                UIElement,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            windows_core::imp::required_hierarchy!(UIElement, DependencyObject);
            impl UIElement {
                pub fn Visibility(&self) -> windows_core::Result<Visibility> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Visibility)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn SetVisibility(&self, value: Visibility) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetVisibility)(
                            windows_core::Interface::as_raw(self),
                            value,
                        )
                        .ok()
                    }
                }
                pub fn PointerMoved<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<Input::PointerRoutedEventArgs>,
                        ) + Send
                        + 'static,
                {
                    let handler = <Input::PointerEventHandler>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(self).PointerMoved)(
                            windows_core::Interface::as_raw(self),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            self.clone(),
                            token__,
                            windows_core::Interface::vtable(self).RemovePointerMoved,
                        ))
                    }
                }
                pub fn PointerExited<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<Input::PointerRoutedEventArgs>,
                        ) + Send
                        + 'static,
                {
                    let handler = <Input::PointerEventHandler>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(self).PointerExited)(
                            windows_core::Interface::as_raw(self),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            self.clone(),
                            token__,
                            windows_core::Interface::vtable(self).RemovePointerExited,
                        ))
                    }
                }
                pub fn Tapped<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<Input::TappedRoutedEventArgs>,
                        ) + Send
                        + 'static,
                {
                    let handler = <Input::TappedEventHandler>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(self).Tapped)(
                            windows_core::Interface::as_raw(self),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            self.clone(),
                            token__,
                            windows_core::Interface::vtable(self).RemoveTapped,
                        ))
                    }
                }
                pub fn DoubleTapped<F>(
                    &self,
                    handler: F,
                ) -> windows_core::Result<windows_core::EventRevoker>
                where
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<Input::DoubleTappedRoutedEventArgs>,
                        ) + Send
                        + 'static,
                {
                    let handler = <Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                        handler(a0, a1);
                        Ok(())
                    });
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        let token__ = (windows_core::Interface::vtable(self).DoubleTapped)(
                            windows_core::Interface::as_raw(self),
                            windows_core::Interface::as_raw(&handler),
                            &mut result__,
                        )
                        .map(|| result__)?;
                        Ok(windows_core::EventRevoker::new(
                            self.clone(),
                            token__,
                            windows_core::Interface::vtable(self).RemoveDoubleTapped,
                        ))
                    }
                }

            }
            impl windows_core::RuntimeType for UIElement {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IUIElement>();
            }
            unsafe impl windows_core::Interface for UIElement {
                type Vtable = <IUIElement as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID = <IUIElement as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for UIElement {
                const NAME: &'static str = "Microsoft.UI.Xaml.UIElement";
            }
            unsafe impl Send for UIElement {}
            unsafe impl Sync for UIElement {}
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
            pub struct Visibility(pub i32);
            impl Visibility {
                pub const Visible: Self = Self(0);
                pub const Collapsed: Self = Self(1);
            }
            impl windows_core::imp::TypeKind for Visibility {
                type TypeKind = windows_core::imp::CopyType;
            }
            impl windows_core::RuntimeType for Visibility {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(
                        b"enum(Microsoft.UI.Xaml.Visibility;i4)",
                    );
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Microsoft.UI.Xaml.Visibility");
            }
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct Window(windows_core::IUnknown);
            windows_core::imp::interface_hierarchy!(
                Window,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl Window {
                pub fn Content(&self) -> windows_core::Result<UIElement> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Content)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
                pub fn SetContent<P0>(&self, value: P0) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<UIElement>,
                {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetContent)(
                            windows_core::Interface::as_raw(self),
                            value.param().abi(),
                        )
                        .ok()
                    }
                }
                pub fn Title(&self) -> windows_core::Result<windows_core::HSTRING> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Title)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| core::mem::transmute(result__))
                    }
                }
                pub fn SetTitle(&self, value: &windows_core::HSTRING) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetTitle)(
                            windows_core::Interface::as_raw(self),
                            core::mem::transmute_copy(value),
                        )
                        .ok()
                    }
                }
                pub fn ExtendsContentIntoTitleBar(&self) -> windows_core::Result<bool> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).ExtendsContentIntoTitleBar)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn SetExtendsContentIntoTitleBar(
                    &self,
                    value: bool,
                ) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetExtendsContentIntoTitleBar)(
                            windows_core::Interface::as_raw(self),
                            value,
                        )
                        .ok()
                    }
                }
                pub fn Activate(&self) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).Activate)(
                            windows_core::Interface::as_raw(self),
                        )
                        .ok()
                    }
                }
                pub fn Close(&self) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).Close)(
                            windows_core::Interface::as_raw(self),
                        )
                        .ok()
                    }
                }
                pub fn SetTitleBar<P0>(&self, titlebar: P0) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<UIElement>,
                {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetTitleBar)(
                            windows_core::Interface::as_raw(self),
                            titlebar.param().abi(),
                        )
                        .ok()
                    }
                }
                pub fn SystemBackdrop(&self) -> windows_core::Result<Media::SystemBackdrop> {
                    let this = &windows_core::Interface::cast::<IWindow2>(self)?;
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).SystemBackdrop)(
                            windows_core::Interface::as_raw(this),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
                pub fn SetSystemBackdrop<P0>(&self, value: P0) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<Media::SystemBackdrop>,
                {
                    let this = &windows_core::Interface::cast::<IWindow2>(self)?;
                    unsafe {
                        (windows_core::Interface::vtable(this).SetSystemBackdrop)(
                            windows_core::Interface::as_raw(this),
                            value.param().abi(),
                        )
                        .ok()
                    }
                }
                pub fn AppWindow(&self) -> windows_core::Result<super::Windowing::AppWindow> {
                    let this = &windows_core::Interface::cast::<IWindow2>(self)?;
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).AppWindow)(
                            windows_core::Interface::as_raw(this),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
                pub fn new() -> windows_core::Result<Self> {
                    Self::IWindowFactory(|this| unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).CreateInstance)(
                            windows_core::Interface::as_raw(this),
                            core::ptr::null_mut(),
                            core::ptr::null_mut(),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    })
                }
                pub fn compose<T>(compose: T) -> windows_core::Result<Self>
                where
                    T: windows_core::Compose,
                {
                    Self::IWindowFactory(|this| unsafe {
                        let (derived__, base__) = windows_core::Compose::compose(compose);
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).CreateInstance)(
                            windows_core::Interface::as_raw(this),
                            core::mem::transmute_copy(&derived__),
                            base__ as *mut _ as _,
                            &mut result__,
                        )
                        .ok()?;
                        let _ = &derived__;
                        windows_core::imp::Type::from_abi(result__)
                    })
                }
                fn IWindowFactory<R, F: FnOnce(&IWindowFactory) -> windows_core::Result<R>>(
                    callback: F,
                ) -> windows_core::Result<R> {
                    static SHARED: windows_core::imp::FactoryCache<Window, IWindowFactory> =
                        windows_core::imp::FactoryCache::new();
                    SHARED.call(callback)
                }

            }
            impl windows_core::RuntimeType for Window {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_class::<Self, IWindow>();
            }
            unsafe impl windows_core::Interface for Window {
                type Vtable = <IWindow as windows_core::Interface>::Vtable;
                const IID: windows_core::GUID = <IWindow as windows_core::Interface>::IID;
            }
            impl windows_core::RuntimeName for Window {
                const NAME: &'static str = "Microsoft.UI.Xaml.Window";
            }
            unsafe impl Send for Window {}
            unsafe impl Sync for Window {}
            pub mod Controls {
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct Border(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    Border,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    Border,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl Border {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            Border,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn Child(&self) -> windows_core::Result<super::UIElement> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).Child)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetChild<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::UIElement>,
                    {
                        unsafe {
                            (windows_core::Interface::vtable(self).SetChild)(
                                windows_core::Interface::as_raw(self),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }

                }
                impl windows_core::RuntimeType for Border {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IBorder>();
                }
                unsafe impl windows_core::Interface for Border {
                    type Vtable = <IBorder as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IBorder as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for Border {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.Border";
                }
                unsafe impl Send for Border {}
                unsafe impl Sync for Border {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct ColumnDefinition(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    ColumnDefinition,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(ColumnDefinition, super::DependencyObject);
                impl ColumnDefinition {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            ColumnDefinition,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn Width(&self) -> windows_core::Result<super::GridLength> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).Width)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: super::GridLength) -> windows_core::Result<()> {
                        unsafe {
                            (windows_core::Interface::vtable(self).SetWidth)(
                                windows_core::Interface::as_raw(self),
                                value,
                            )
                            .ok()
                        }
                    }

                }
                impl windows_core::RuntimeType for ColumnDefinition {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IColumnDefinition>();
                }
                unsafe impl windows_core::Interface for ColumnDefinition {
                    type Vtable = <IColumnDefinition as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <IColumnDefinition as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for ColumnDefinition {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ColumnDefinition";
                }
                unsafe impl Send for ColumnDefinition {}
                unsafe impl Sync for ColumnDefinition {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct ContentControl(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    ContentControl,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    ContentControl,
                    Control,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl ContentControl {
                    pub fn Content(&self) -> windows_core::Result<windows_core::IInspectable> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).Content)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetContent<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<windows_core::IInspectable>,
                    {
                        unsafe {
                            (windows_core::Interface::vtable(self).SetContent)(
                                windows_core::Interface::as_raw(self),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn Background(&self) -> windows_core::Result<super::Media::Brush> {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Background)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::Media::Brush>,
                    {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetBackground)(
                                windows_core::Interface::as_raw(this),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }


                }
                impl windows_core::RuntimeType for ContentControl {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IContentControl>();
                }
                unsafe impl windows_core::Interface for ContentControl {
                    type Vtable = <IContentControl as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <IContentControl as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for ContentControl {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ContentControl";
                }
                unsafe impl Send for ContentControl {}
                unsafe impl Sync for ContentControl {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct Control(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    Control,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    Control,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl Control {
                    pub fn Background(&self) -> windows_core::Result<super::Media::Brush> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).Background)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::Media::Brush>,
                    {
                        unsafe {
                            (windows_core::Interface::vtable(self).SetBackground)(
                                windows_core::Interface::as_raw(self),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }


                }
                impl windows_core::RuntimeType for Control {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IControl>();
                }
                unsafe impl windows_core::Interface for Control {
                    type Vtable = <IControl as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IControl as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for Control {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.Control";
                }
                unsafe impl Send for Control {}
                unsafe impl Sync for Control {}
                windows_core::imp::define_interface!(
                    IBorder,
                    IBorder_Vtbl,
                    0x1ca13b47_ff5c_5abc_a411_a177df9482a9
                );
                impl windows_core::RuntimeType for IBorder {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IBorder",
                        );
                }
                #[repr(C)]
                pub struct IBorder_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    BorderBrush: usize,
                    SetBorderBrush: usize,
                    BorderThickness: usize,
                    SetBorderThickness: usize,
                    Background: usize,
                    SetBackground: usize,
                    BackgroundSizing: usize,
                    SetBackgroundSizing: usize,
                    CornerRadius: usize,
                    SetCornerRadius: usize,
                    Padding: usize,
                    SetPadding: usize,
                    pub Child: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                    pub SetChild: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IColumnDefinition,
                    IColumnDefinition_Vtbl,
                    0x454cea14_87ec_5890_bb62_f1d82a94758e
                );
                impl windows_core::RuntimeType for IColumnDefinition {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IColumnDefinition",
                        );
                }
                #[repr(C)]
                pub struct IColumnDefinition_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    pub Width: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut super::GridLength,
                    )
                        -> windows_core::HRESULT,
                    pub SetWidth: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        super::GridLength,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IContentControl,
                    IContentControl_Vtbl,
                    0x07e81761_11b2_52ae_8f8b_4d53d2b5900a
                );
                impl windows_core::RuntimeType for IContentControl {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IContentControl",
                        );
                }
                #[repr(C)]
                pub struct IContentControl_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    pub Content: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                    pub SetContent: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IControl,
                    IControl_Vtbl,
                    0x857d6e8a_d45a_5c69_a99c_bf6a5c54fb38
                );
                impl windows_core::RuntimeType for IControl {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IControl",
                        );
                }
                #[repr(C)]
                pub struct IControl_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    IsFocusEngagementEnabled: usize,
                    SetIsFocusEngagementEnabled: usize,
                    IsFocusEngaged: usize,
                    SetIsFocusEngaged: usize,
                    RequiresPointer: usize,
                    SetRequiresPointer: usize,
                    FontSize: usize,
                    SetFontSize: usize,
                    FontFamily: usize,
                    SetFontFamily: usize,
                    FontWeight: usize,
                    SetFontWeight: usize,
                    FontStyle: usize,
                    SetFontStyle: usize,
                    FontStretch: usize,
                    SetFontStretch: usize,
                    CharacterSpacing: usize,
                    SetCharacterSpacing: usize,
                    Foreground: usize,
                    SetForeground: usize,
                    IsTextScaleFactorEnabled: usize,
                    SetIsTextScaleFactorEnabled: usize,
                    IsEnabled: usize,
                    SetIsEnabled: usize,
                    TabNavigation: usize,
                    SetTabNavigation: usize,
                    Template: usize,
                    SetTemplate: usize,
                    Padding: usize,
                    SetPadding: usize,
                    HorizontalContentAlignment: usize,
                    SetHorizontalContentAlignment: usize,
                    VerticalContentAlignment: usize,
                    SetVerticalContentAlignment: usize,
                    pub Background: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                    pub SetBackground: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IImage,
                    IImage_Vtbl,
                    0x220d3d8d_66de_53a1_a215_ba9c165565ab
                );
                impl windows_core::RuntimeType for IImage {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IImage",
                        );
                }
                #[repr(C)]
                pub struct IImage_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    pub Source: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                    pub SetSource: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IItemsControl,
                    IItemsControl_Vtbl,
                    0xbf1ccb54_83e2_5b98_acbc_736f876c3d35
                );
                impl windows_core::RuntimeType for IItemsControl {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IItemsControl",
                        );
                }
                #[repr(C)]
                pub struct IItemsControl_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                windows_core::imp::define_interface!(
                    IMenuFlyoutItem,
                    IMenuFlyoutItem_Vtbl,
                    0x4252df5a_44f9_5ee8_b1cc_53de9aaa4095
                );
                impl windows_core::RuntimeType for IMenuFlyoutItem {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IMenuFlyoutItem",
                        );
                }
                #[repr(C)]
                pub struct IMenuFlyoutItem_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    Text: usize,
                    SetText: usize,
                    Command: usize,
                    SetCommand: usize,
                    CommandParameter: usize,
                    SetCommandParameter: usize,
                    Icon: usize,
                    SetIcon: usize,
                    KeyboardAcceleratorTextOverride: usize,
                    SetKeyboardAcceleratorTextOverride: usize,
                    TemplateSettings: usize,
                    pub Click: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut i64,
                    )
                        -> windows_core::HRESULT,
                    pub RemoveClick: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        i64,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IMenuFlyoutItemBase,
                    IMenuFlyoutItemBase_Vtbl,
                    0x4bee2715_44a1_5f94_86e8_02ddbe3dc6b9
                );
                impl windows_core::RuntimeType for IMenuFlyoutItemBase {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IMenuFlyoutItemBase",
                        );
                }
                #[repr(C)]
                pub struct IMenuFlyoutItemBase_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                windows_core::imp::define_interface!(
                    IProgressBar,
                    IProgressBar_Vtbl,
                    0x87555c8c_0aaf_52c1_8390_0db17f40438e
                );
                impl windows_core::RuntimeType for IProgressBar {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IProgressBar",
                        );
                }
                #[repr(C)]
                pub struct IProgressBar_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    pub IsIndeterminate: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut bool,
                    )
                        -> windows_core::HRESULT,
                    pub SetIsIndeterminate: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        bool,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IRowDefinition,
                    IRowDefinition_Vtbl,
                    0xfe870f2f_89ef_5dac_9f33_968d0dc577c3
                );
                impl windows_core::RuntimeType for IRowDefinition {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IRowDefinition",
                        );
                }
                #[repr(C)]
                pub struct IRowDefinition_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    pub Height: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut super::GridLength,
                    )
                        -> windows_core::HRESULT,
                    pub SetHeight: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        super::GridLength,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IScrollViewer,
                    IScrollViewer_Vtbl,
                    0x1dc28c2e_996c_5394_89c3_4dc656b4ad46
                );
                impl windows_core::RuntimeType for IScrollViewer {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IScrollViewer",
                        );
                }
                #[repr(C)]
                pub struct IScrollViewer_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    HorizontalScrollBarVisibility: usize,
                    SetHorizontalScrollBarVisibility: usize,
                    VerticalScrollBarVisibility: usize,
                    SetVerticalScrollBarVisibility: usize,
                    IsHorizontalRailEnabled: usize,
                    SetIsHorizontalRailEnabled: usize,
                    IsVerticalRailEnabled: usize,
                    SetIsVerticalRailEnabled: usize,
                    IsHorizontalScrollChainingEnabled: usize,
                    SetIsHorizontalScrollChainingEnabled: usize,
                    IsVerticalScrollChainingEnabled: usize,
                    SetIsVerticalScrollChainingEnabled: usize,
                    IsZoomChainingEnabled: usize,
                    SetIsZoomChainingEnabled: usize,
                    IsScrollInertiaEnabled: usize,
                    SetIsScrollInertiaEnabled: usize,
                    IsZoomInertiaEnabled: usize,
                    SetIsZoomInertiaEnabled: usize,
                    HorizontalScrollMode: usize,
                    SetHorizontalScrollMode: usize,
                    VerticalScrollMode: usize,
                    SetVerticalScrollMode: usize,
                    ZoomMode: usize,
                    SetZoomMode: usize,
                    HorizontalSnapPointsAlignment: usize,
                    SetHorizontalSnapPointsAlignment: usize,
                    VerticalSnapPointsAlignment: usize,
                    SetVerticalSnapPointsAlignment: usize,
                    HorizontalSnapPointsType: usize,
                    SetHorizontalSnapPointsType: usize,
                    VerticalSnapPointsType: usize,
                    SetVerticalSnapPointsType: usize,
                    ZoomSnapPointsType: usize,
                    SetZoomSnapPointsType: usize,
                    HorizontalOffset: usize,
                    ViewportWidth: usize,
                    ScrollableWidth: usize,
                    ComputedHorizontalScrollBarVisibility: usize,
                    ExtentWidth: usize,
                    pub VerticalOffset: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut f64,
                    )
                        -> windows_core::HRESULT,
                    ViewportHeight: usize,
                    ScrollableHeight: usize,
                    ComputedVerticalScrollBarVisibility: usize,
                    ExtentHeight: usize,
                    MinZoomFactor: usize,
                    SetMinZoomFactor: usize,
                    MaxZoomFactor: usize,
                    SetMaxZoomFactor: usize,
                    ZoomFactor: usize,
                    ZoomSnapPoints: usize,
                    TopLeftHeader: usize,
                    SetTopLeftHeader: usize,
                    LeftHeader: usize,
                    SetLeftHeader: usize,
                    TopHeader: usize,
                    SetTopHeader: usize,
                    ReduceViewportForCoreInputViewOcclusions: usize,
                    SetReduceViewportForCoreInputViewOcclusions: usize,
                    HorizontalAnchorRatio: usize,
                    SetHorizontalAnchorRatio: usize,
                    VerticalAnchorRatio: usize,
                    SetVerticalAnchorRatio: usize,
                    CanContentRenderOutsideBounds: usize,
                    SetCanContentRenderOutsideBounds: usize,
                    AnchorRequested: usize,
                    RemoveAnchorRequested: usize,
                    ViewChanging: usize,
                    RemoveViewChanging: usize,
                    ViewChanged: usize,
                    RemoveViewChanged: usize,
                    DirectManipulationStarted: usize,
                    RemoveDirectManipulationStarted: usize,
                    DirectManipulationCompleted: usize,
                    RemoveDirectManipulationCompleted: usize,
                    ScrollToHorizontalOffset: usize,
                    ScrollToVerticalOffset: usize,
                    ZoomToFactor: usize,
                    pub ChangeView: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut bool,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    ISelectionChangedEventArgs,
                    ISelectionChangedEventArgs_Vtbl,
                    0xb6c18076_4b76_5416_ad29_e2dc20c46246
                );
                impl windows_core::RuntimeType for ISelectionChangedEventArgs {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.ISelectionChangedEventArgs",
                        );
                }
                #[repr(C)]
                pub struct ISelectionChangedEventArgs_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                windows_core::imp::define_interface!(
                    ITextBlock,
                    ITextBlock_Vtbl,
                    0x1ac8d84f_392c_5c7e_83f5_a53e3bf0abb0
                );
                impl windows_core::RuntimeType for ITextBlock {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.ITextBlock",
                        );
                }
                #[repr(C)]
                pub struct ITextBlock_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    FontSize: usize,
                    SetFontSize: usize,
                    FontFamily: usize,
                    SetFontFamily: usize,
                    FontWeight: usize,
                    SetFontWeight: usize,
                    FontStyle: usize,
                    SetFontStyle: usize,
                    FontStretch: usize,
                    SetFontStretch: usize,
                    CharacterSpacing: usize,
                    SetCharacterSpacing: usize,
                    Foreground: usize,
                    SetForeground: usize,
                    TextWrapping: usize,
                    SetTextWrapping: usize,
                    TextTrimming: usize,
                    SetTextTrimming: usize,
                    TextAlignment: usize,
                    SetTextAlignment: usize,
                    pub Text: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                    pub SetText: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IToggleMenuFlyoutItem,
                    IToggleMenuFlyoutItem_Vtbl,
                    0x1803f260_67e4_5bc1_a63a_123510167bb8
                );
                impl windows_core::RuntimeType for IToggleMenuFlyoutItem {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IToggleMenuFlyoutItem",
                        );
                }
                #[repr(C)]
                pub struct IToggleMenuFlyoutItem_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    pub IsChecked: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut bool,
                    )
                        -> windows_core::HRESULT,
                    pub SetIsChecked: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        bool,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IXamlControlsResources,
                    IXamlControlsResources_Vtbl,
                    0x918ca043_f42c_5805_861b_62d6d1d0c162
                );
                impl windows_core::RuntimeType for IXamlControlsResources {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Controls.IXamlControlsResources",
                        );
                }
                #[repr(C)]
                pub struct IXamlControlsResources_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct Image(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    Image,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    Image,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl Image {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            Image,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn Source(&self) -> windows_core::Result<super::Media::ImageSource> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).Source)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetSource<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::Media::ImageSource>,
                    {
                        unsafe {
                            (windows_core::Interface::vtable(self).SetSource)(
                                windows_core::Interface::as_raw(self),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }

                }
                impl windows_core::RuntimeType for Image {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IImage>();
                }
                unsafe impl windows_core::Interface for Image {
                    type Vtable = <IImage as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IImage as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for Image {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.Image";
                }
                unsafe impl Send for Image {}
                unsafe impl Sync for Image {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct ItemsControl(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    ItemsControl,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    ItemsControl,
                    Control,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl ItemsControl {
                    pub fn Background(&self) -> windows_core::Result<super::Media::Brush> {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Background)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::Media::Brush>,
                    {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetBackground)(
                                windows_core::Interface::as_raw(this),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }


                }
                impl windows_core::RuntimeType for ItemsControl {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IItemsControl>();
                }
                unsafe impl windows_core::Interface for ItemsControl {
                    type Vtable = <IItemsControl as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IItemsControl as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for ItemsControl {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ItemsControl";
                }
                unsafe impl Send for ItemsControl {}
                unsafe impl Sync for ItemsControl {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct MenuFlyoutItem(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    MenuFlyoutItem,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    MenuFlyoutItem,
                    MenuFlyoutItemBase,
                    Control,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl MenuFlyoutItem {
                    pub fn Background(&self) -> windows_core::Result<super::Media::Brush> {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Background)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::Media::Brush>,
                    {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetBackground)(
                                windows_core::Interface::as_raw(this),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn Click<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::RoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let handler = <super::RoutedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(self).Click)(
                                windows_core::Interface::as_raw(self),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                self.clone(),
                                token__,
                                windows_core::Interface::vtable(self).RemoveClick,
                            ))
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }


                }
                impl windows_core::RuntimeType for MenuFlyoutItem {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IMenuFlyoutItem>();
                }
                unsafe impl windows_core::Interface for MenuFlyoutItem {
                    type Vtable = <IMenuFlyoutItem as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <IMenuFlyoutItem as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for MenuFlyoutItem {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.MenuFlyoutItem";
                }
                unsafe impl Send for MenuFlyoutItem {}
                unsafe impl Sync for MenuFlyoutItem {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct MenuFlyoutItemBase(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    MenuFlyoutItemBase,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    MenuFlyoutItemBase,
                    Control,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl MenuFlyoutItemBase {
                    pub fn Background(&self) -> windows_core::Result<super::Media::Brush> {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Background)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::Media::Brush>,
                    {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetBackground)(
                                windows_core::Interface::as_raw(this),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }
                }
                impl windows_core::RuntimeType for MenuFlyoutItemBase {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IMenuFlyoutItemBase>();
                }
                unsafe impl windows_core::Interface for MenuFlyoutItemBase {
                    type Vtable = <IMenuFlyoutItemBase as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <IMenuFlyoutItemBase as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for MenuFlyoutItemBase {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.MenuFlyoutItemBase";
                }
                unsafe impl Send for MenuFlyoutItemBase {}
                unsafe impl Sync for MenuFlyoutItemBase {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct ProgressBar(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    ProgressBar,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    ProgressBar,
                    Primitives::RangeBase,
                    Control,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl ProgressBar {
                    pub fn Background(&self) -> windows_core::Result<super::Media::Brush> {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Background)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::Media::Brush>,
                    {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetBackground)(
                                windows_core::Interface::as_raw(this),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn IsIndeterminate(&self) -> windows_core::Result<bool> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).IsIndeterminate)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetIsIndeterminate(&self, value: bool) -> windows_core::Result<()> {
                        unsafe {
                            (windows_core::Interface::vtable(self).SetIsIndeterminate)(
                                windows_core::Interface::as_raw(self),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn Value(&self) -> windows_core::Result<f64> {
                        let this = &windows_core::Interface::cast::<Primitives::IRangeBase>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Value)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetValue(&self, value: f64) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<Primitives::IRangeBase>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetValue)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }


                }
                impl windows_core::RuntimeType for ProgressBar {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IProgressBar>();
                }
                unsafe impl windows_core::Interface for ProgressBar {
                    type Vtable = <IProgressBar as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IProgressBar as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for ProgressBar {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ProgressBar";
                }
                unsafe impl Send for ProgressBar {}
                unsafe impl Sync for ProgressBar {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct RowDefinition(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    RowDefinition,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(RowDefinition, super::DependencyObject);
                impl RowDefinition {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            RowDefinition,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn Height(&self) -> windows_core::Result<super::GridLength> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).Height)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetHeight(&self, value: super::GridLength) -> windows_core::Result<()> {
                        unsafe {
                            (windows_core::Interface::vtable(self).SetHeight)(
                                windows_core::Interface::as_raw(self),
                                value,
                            )
                            .ok()
                        }
                    }

                }
                impl windows_core::RuntimeType for RowDefinition {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IRowDefinition>();
                }
                unsafe impl windows_core::Interface for RowDefinition {
                    type Vtable = <IRowDefinition as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <IRowDefinition as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for RowDefinition {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.RowDefinition";
                }
                unsafe impl Send for RowDefinition {}
                unsafe impl Sync for RowDefinition {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct ScrollViewer(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    ScrollViewer,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    ScrollViewer,
                    ContentControl,
                    Control,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl ScrollViewer {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            ScrollViewer,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn Content(&self) -> windows_core::Result<windows_core::IInspectable> {
                        let this = &windows_core::Interface::cast::<IContentControl>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Content)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetContent<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<windows_core::IInspectable>,
                    {
                        let this = &windows_core::Interface::cast::<IContentControl>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetContent)(
                                windows_core::Interface::as_raw(this),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn Background(&self) -> windows_core::Result<super::Media::Brush> {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Background)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::Media::Brush>,
                    {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetBackground)(
                                windows_core::Interface::as_raw(this),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn VerticalOffset(&self) -> windows_core::Result<f64> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).VerticalOffset)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ChangeView(
                        &self,
                        horizontaloffset: Option<f64>,
                        verticaloffset: Option<f64>,
                        zoomfactor: Option<f32>,
                    ) -> windows_core::Result<bool> {
                        let horizontaloffset__ = horizontaloffset
                            .map(<windows_reference::IReference<f64> as From<_>>::from);
                        let verticaloffset__ = verticaloffset
                            .map(<windows_reference::IReference<f64> as From<_>>::from);
                        let zoomfactor__ =
                            zoomfactor.map(<windows_reference::IReference<f32> as From<_>>::from);
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).ChangeView)(
                                windows_core::Interface::as_raw(self),
                                windows_core::Param::param(horizontaloffset__.as_ref()).abi(),
                                windows_core::Param::param(verticaloffset__.as_ref()).abi(),
                                windows_core::Param::param(zoomfactor__.as_ref()).abi(),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }

                }
                impl windows_core::RuntimeType for ScrollViewer {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IScrollViewer>();
                }
                unsafe impl windows_core::Interface for ScrollViewer {
                    type Vtable = <IScrollViewer as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IScrollViewer as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for ScrollViewer {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ScrollViewer";
                }
                unsafe impl Send for ScrollViewer {}
                unsafe impl Sync for ScrollViewer {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct SelectionChangedEventArgs(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    SelectionChangedEventArgs,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    SelectionChangedEventArgs,
                    super::RoutedEventArgs
                );
                impl SelectionChangedEventArgs {

                }
                impl windows_core::RuntimeType for SelectionChangedEventArgs {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, ISelectionChangedEventArgs>(
                        );
                }
                unsafe impl windows_core::Interface for SelectionChangedEventArgs {
                    type Vtable = <ISelectionChangedEventArgs as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <ISelectionChangedEventArgs as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for SelectionChangedEventArgs {
                    const NAME: &'static str =
                        "Microsoft.UI.Xaml.Controls.SelectionChangedEventArgs";
                }
                unsafe impl Send for SelectionChangedEventArgs {}
                unsafe impl Sync for SelectionChangedEventArgs {}
                windows_core::imp::define_interface!(
                    SelectionChangedEventHandler,
                    SelectionChangedEventHandler_Vtbl,
                    0xa232390d_0e34_595e_8931_fa928a9909f4
                );
                impl windows_core::RuntimeType for SelectionChangedEventHandler {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                }
                impl SelectionChangedEventHandler {
                    pub fn new<
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<SelectionChangedEventArgs>,
                            ) -> windows_core::Result<()>
                            + Send
                            + 'static,
                    >(
                        invoke: F,
                    ) -> Self {
                        let com = windows_core::imp::DelegateBox::<Self, F>::new(
                            &SelectionChangedEventHandlerBox::<F>::VTABLE,
                            invoke,
                        );
                        unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
                    }
                    pub fn Invoke<P0, P1>(&self, sender: P0, e: P1) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<windows_core::IInspectable>,
                        P1: windows_core::Param<SelectionChangedEventArgs>,
                    {
                        unsafe {
                            (windows_core::Interface::vtable(self).Invoke)(
                                windows_core::Interface::as_raw(self),
                                sender.param().abi(),
                                e.param().abi(),
                            )
                            .ok()
                        }
                    }
                }
                #[repr(C)]
                pub struct SelectionChangedEventHandler_Vtbl {
                    base__: windows_core::IUnknown_Vtbl,
                    Invoke: unsafe extern "system" fn(
                        this: *mut core::ffi::c_void,
                        sender: *mut core::ffi::c_void,
                        e: *mut core::ffi::c_void,
                    ) -> windows_core::HRESULT,
                }
                struct SelectionChangedEventHandlerBox<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<SelectionChangedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                >(core::marker::PhantomData<(fn() -> F,)>);
                impl<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<SelectionChangedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                > SelectionChangedEventHandlerBox<F>
                {
                    const VTABLE: SelectionChangedEventHandler_Vtbl =
                        SelectionChangedEventHandler_Vtbl {
                            base__: windows_core::IUnknown_Vtbl {
                                QueryInterface: windows_core::imp::DelegateBox::<
                                    SelectionChangedEventHandler,
                                    F,
                                >::QueryInterface,
                                AddRef: windows_core::imp::DelegateBox::<
                                    SelectionChangedEventHandler,
                                    F,
                                >::AddRef,
                                Release: windows_core::imp::DelegateBox::<
                                    SelectionChangedEventHandler,
                                    F,
                                >::Release,
                            },
                            Invoke: Self::Invoke,
                        };
                    unsafe extern "system" fn Invoke(
                        this: *mut core::ffi::c_void,
                        sender: *mut core::ffi::c_void,
                        e: *mut core::ffi::c_void,
                    ) -> windows_core::HRESULT {
                        unsafe {
                            let this = &mut *(this as *mut *mut core::ffi::c_void
                                as *mut windows_core::imp::DelegateBox<
                                    SelectionChangedEventHandler,
                                    F,
                                >);
                            (this.invoke)(
                                core::mem::transmute_copy(&sender),
                                core::mem::transmute_copy(&e),
                            )
                            .into()
                        }
                    }
                }
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct TextBlock(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    TextBlock,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    TextBlock,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl TextBlock {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            TextBlock,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn Text(&self) -> windows_core::Result<windows_core::HSTRING> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).Text)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .map(|| core::mem::transmute(result__))
                        }
                    }
                    pub fn SetText(
                        &self,
                        value: &windows_core::HSTRING,
                    ) -> windows_core::Result<()> {
                        unsafe {
                            (windows_core::Interface::vtable(self).SetText)(
                                windows_core::Interface::as_raw(self),
                                core::mem::transmute_copy(value),
                            )
                            .ok()
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }

                }
                impl windows_core::RuntimeType for TextBlock {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, ITextBlock>();
                }
                unsafe impl windows_core::Interface for TextBlock {
                    type Vtable = <ITextBlock as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <ITextBlock as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for TextBlock {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.TextBlock";
                }
                unsafe impl Send for TextBlock {}
                unsafe impl Sync for TextBlock {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct ToggleMenuFlyoutItem(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    ToggleMenuFlyoutItem,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    ToggleMenuFlyoutItem,
                    MenuFlyoutItem,
                    MenuFlyoutItemBase,
                    Control,
                    super::FrameworkElement,
                    super::UIElement,
                    super::DependencyObject
                );
                impl ToggleMenuFlyoutItem {
                    pub fn Background(&self) -> windows_core::Result<super::Media::Brush> {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Background)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<super::Media::Brush>,
                    {
                        let this = &windows_core::Interface::cast::<IControl>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetBackground)(
                                windows_core::Interface::as_raw(this),
                                value.param().abi(),
                            )
                            .ok()
                        }
                    }
                    pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualWidth)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).ActualHeight)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn Width(&self) -> windows_core::Result<f64> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Width)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetWidth)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn SizeChanged<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::SizeChangedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        let handler = <super::SizeChangedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveSizeChanged,
                            ))
                        }
                    }
                    pub fn FindName(
                        &self,
                        name: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        let this =
                            &windows_core::Interface::cast::<super::IFrameworkElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).FindName)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(name),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn Click<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::RoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<IMenuFlyoutItem>(self)?;
                        let handler = <super::RoutedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Click)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveClick,
                            ))
                        }
                    }
                    pub fn IsChecked(&self) -> windows_core::Result<bool> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).IsChecked)(
                                windows_core::Interface::as_raw(self),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetIsChecked(&self, value: bool) -> windows_core::Result<()> {
                        unsafe {
                            (windows_core::Interface::vtable(self).SetIsChecked)(
                                windows_core::Interface::as_raw(self),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn Visibility(&self) -> windows_core::Result<super::Visibility> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Visibility)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                    pub fn SetVisibility(
                        &self,
                        value: super::Visibility,
                    ) -> windows_core::Result<()> {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        unsafe {
                            (windows_core::Interface::vtable(this).SetVisibility)(
                                windows_core::Interface::as_raw(this),
                                value,
                            )
                            .ok()
                        }
                    }
                    pub fn PointerMoved<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerMoved,
                            ))
                        }
                    }
                    pub fn PointerExited<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::PointerRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::PointerEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).PointerExited)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemovePointerExited,
                            ))
                        }
                    }
                    pub fn Tapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::TappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler = <super::Input::TappedEventHandler>::new(move |a0, a1| {
                            handler(a0, a1);
                            Ok(())
                        });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveTapped,
                            ))
                        }
                    }
                    pub fn DoubleTapped<F>(
                        &self,
                        handler: F,
                    ) -> windows_core::Result<windows_core::EventRevoker>
                    where
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<super::Input::DoubleTappedRoutedEventArgs>,
                            ) + Send
                            + 'static,
                    {
                        let this = &windows_core::Interface::cast::<super::IUIElement>(self)?;
                        let handler =
                            <super::Input::DoubleTappedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                windows_core::Interface::as_raw(this),
                                windows_core::Interface::as_raw(&handler),
                                &mut result__,
                            )
                            .map(|| result__)?;
                            Ok(windows_core::EventRevoker::new(
                                this.clone(),
                                token__,
                                windows_core::Interface::vtable(this).RemoveDoubleTapped,
                            ))
                        }
                    }


                }
                impl windows_core::RuntimeType for ToggleMenuFlyoutItem {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IToggleMenuFlyoutItem>();
                }
                unsafe impl windows_core::Interface for ToggleMenuFlyoutItem {
                    type Vtable = <IToggleMenuFlyoutItem as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <IToggleMenuFlyoutItem as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for ToggleMenuFlyoutItem {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ToggleMenuFlyoutItem";
                }
                unsafe impl Send for ToggleMenuFlyoutItem {}
                unsafe impl Sync for ToggleMenuFlyoutItem {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct XamlControlsResources(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    XamlControlsResources,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    XamlControlsResources,
                    super::ResourceDictionary,
                    super::DependencyObject
                );
                impl XamlControlsResources {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            XamlControlsResources,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn First(
                        &self,
                    ) -> windows_core::Result<
                        windows_collections::IIterator<
                            windows_collections::IKeyValuePair<
                                windows_core::IInspectable,
                                windows_core::IInspectable,
                            >,
                        >,
                    > {
                        let this = &windows_core::Interface::cast::<
                            super::super::super::super::Windows::Foundation::Collections::IIterable<
                                windows_collections::IKeyValuePair<
                                    windows_core::IInspectable,
                                    windows_core::IInspectable,
                                >,
                            >,
                        >(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).First)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn MergedDictionaries(
                        &self,
                    ) -> windows_core::Result<
                        super::super::super::super::Windows::Foundation::Collections::IVector<
                            super::ResourceDictionary,
                        >,
                    > {
                        let this =
                            &windows_core::Interface::cast::<super::IResourceDictionary>(self)?;
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).MergedDictionaries)(
                                windows_core::Interface::as_raw(this),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }

                }
                impl windows_core::RuntimeType for XamlControlsResources {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IXamlControlsResources>();
                }
                unsafe impl windows_core::Interface for XamlControlsResources {
                    type Vtable = <IXamlControlsResources as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <IXamlControlsResources as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for XamlControlsResources {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.XamlControlsResources";
                }
                unsafe impl Send for XamlControlsResources {}
                unsafe impl Sync for XamlControlsResources {}
                impl IntoIterator for XamlControlsResources {
                    type Item = windows_collections::IKeyValuePair<
                        windows_core::IInspectable,
                        windows_core::IInspectable,
                    >;
                    type IntoIter = windows_collections::BufferedIterator<Self::Item>;
                    fn into_iter(self) -> Self::IntoIter {
                        IntoIterator::into_iter(&self)
                    }
                }
                impl IntoIterator for &XamlControlsResources {
                    type Item = windows_collections::IKeyValuePair<
                        windows_core::IInspectable,
                        windows_core::IInspectable,
                    >;
                    type IntoIter = windows_collections::BufferedIterator<Self::Item>;
                    fn into_iter(self) -> Self::IntoIter {
                        windows_collections::BufferedIterator::new(self.First().unwrap())
                    }
                }
                pub mod Primitives {
                    #[repr(transparent)]
                    #[derive(Clone, Debug, Eq, PartialEq)]
                    pub struct ButtonBase(windows_core::IUnknown);
                    windows_core::imp::interface_hierarchy!(
                        ButtonBase,
                        windows_core::IUnknown,
                        windows_core::IInspectable
                    );
                    windows_core::imp::required_hierarchy!(
                        ButtonBase,
                        super::ContentControl,
                        super::Control,
                        super::super::FrameworkElement,
                        super::super::UIElement,
                        super::super::DependencyObject
                    );
                    impl ButtonBase {
                        pub fn Click<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::RoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let handler = <super::super::RoutedEventHandler>::new(move |a0, a1| {
                                handler(a0, a1);
                                Ok(())
                            });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(self).Click)(
                                    windows_core::Interface::as_raw(self),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    self.clone(),
                                    token__,
                                    windows_core::Interface::vtable(self).RemoveClick,
                                ))
                            }
                        }
                        pub fn Content(&self) -> windows_core::Result<windows_core::IInspectable> {
                            let this =
                                &windows_core::Interface::cast::<super::IContentControl>(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Content)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .and_then(|| windows_core::imp::Type::from_abi(result__))
                            }
                        }
                        pub fn SetContent<P0>(&self, value: P0) -> windows_core::Result<()>
                        where
                            P0: windows_core::Param<windows_core::IInspectable>,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::IContentControl>(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetContent)(
                                    windows_core::Interface::as_raw(this),
                                    value.param().abi(),
                                )
                                .ok()
                            }
                        }
                        pub fn Background(
                            &self,
                        ) -> windows_core::Result<super::super::Media::Brush>
                        {
                            let this = &windows_core::Interface::cast::<super::IControl>(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Background)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .and_then(|| windows_core::imp::Type::from_abi(result__))
                            }
                        }
                        pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                        where
                            P0: windows_core::Param<super::super::Media::Brush>,
                        {
                            let this = &windows_core::Interface::cast::<super::IControl>(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetBackground)(
                                    windows_core::Interface::as_raw(this),
                                    value.param().abi(),
                                )
                                .ok()
                            }
                        }
                        pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).ActualWidth)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).ActualHeight)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn Width(&self) -> windows_core::Result<f64> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Width)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetWidth)(
                                    windows_core::Interface::as_raw(this),
                                    value,
                                )
                                .ok()
                            }
                        }
                        pub fn SizeChanged<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::SizeChangedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            let handler =
                                <super::super::SizeChangedEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemoveSizeChanged,
                                ))
                            }
                        }
                        pub fn FindName(
                            &self,
                            name: &windows_core::HSTRING,
                        ) -> windows_core::Result<windows_core::IInspectable>
                        {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).FindName)(
                                    windows_core::Interface::as_raw(this),
                                    core::mem::transmute_copy(name),
                                    &mut result__,
                                )
                                .and_then(|| windows_core::imp::Type::from_abi(result__))
                            }
                        }
                        pub fn Visibility(&self) -> windows_core::Result<super::super::Visibility> {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Visibility)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn SetVisibility(
                            &self,
                            value: super::super::Visibility,
                        ) -> windows_core::Result<()> {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetVisibility)(
                                    windows_core::Interface::as_raw(this),
                                    value,
                                )
                                .ok()
                            }
                        }
                        pub fn PointerMoved<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::Input::PointerRoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler =
                                <super::super::Input::PointerEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemovePointerMoved,
                                ))
                            }
                        }
                        pub fn PointerExited<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::Input::PointerRoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler =
                                <super::super::Input::PointerEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ =
                                    (windows_core::Interface::vtable(this).PointerExited)(
                                        windows_core::Interface::as_raw(this),
                                        windows_core::Interface::as_raw(&handler),
                                        &mut result__,
                                    )
                                    .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemovePointerExited,
                                ))
                            }
                        }
                        pub fn Tapped<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::Input::TappedRoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler =
                                <super::super::Input::TappedEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemoveTapped,
                                ))
                            }
                        }
                        pub fn DoubleTapped<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<
                                        super::super::Input::DoubleTappedRoutedEventArgs,
                                    >,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler = <super::super::Input::DoubleTappedEventHandler>::new(
                                move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                },
                            );
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemoveDoubleTapped,
                                ))
                            }
                        }


                    }
                    impl windows_core::RuntimeType for ButtonBase {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_class::<Self, IButtonBase>();
                    }
                    unsafe impl windows_core::Interface for ButtonBase {
                        type Vtable = <IButtonBase as windows_core::Interface>::Vtable;
                        const IID: windows_core::GUID =
                            <IButtonBase as windows_core::Interface>::IID;
                    }
                    impl windows_core::RuntimeName for ButtonBase {
                        const NAME: &'static str =
                            "Microsoft.UI.Xaml.Controls.Primitives.ButtonBase";
                    }
                    unsafe impl Send for ButtonBase {}
                    unsafe impl Sync for ButtonBase {}
                    windows_core::imp::define_interface!(
                        IButtonBase,
                        IButtonBase_Vtbl,
                        0x65714269_2473_5327_a652_0ea6bce7f403
                    );
                    impl windows_core::RuntimeType for IButtonBase {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_interface::<Self>();
                        const NAME: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::from_slice(
                                b"Microsoft.UI.Xaml.Controls.Primitives.IButtonBase",
                            );
                    }
                    #[repr(C)]
                    pub struct IButtonBase_Vtbl {
                        pub base__: windows_core::IInspectable_Vtbl,
                        ClickMode: usize,
                        SetClickMode: usize,
                        IsPointerOver: usize,
                        IsPressed: usize,
                        Command: usize,
                        SetCommand: usize,
                        CommandParameter: usize,
                        SetCommandParameter: usize,
                        pub Click: unsafe extern "system" fn(
                            *mut core::ffi::c_void,
                            *mut core::ffi::c_void,
                            *mut i64,
                        )
                            -> windows_core::HRESULT,
                        pub RemoveClick: unsafe extern "system" fn(
                            *mut core::ffi::c_void,
                            i64,
                        )
                            -> windows_core::HRESULT,
                    }
                    windows_core::imp::define_interface!(
                        IRangeBase,
                        IRangeBase_Vtbl,
                        0x540d6d61_8fac_5d5c_b5b0_e172a7dde103
                    );
                    impl windows_core::RuntimeType for IRangeBase {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_interface::<Self>();
                        const NAME: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::from_slice(
                                b"Microsoft.UI.Xaml.Controls.Primitives.IRangeBase",
                            );
                    }
                    #[repr(C)]
                    pub struct IRangeBase_Vtbl {
                        pub base__: windows_core::IInspectable_Vtbl,
                        Minimum: usize,
                        SetMinimum: usize,
                        Maximum: usize,
                        SetMaximum: usize,
                        SmallChange: usize,
                        SetSmallChange: usize,
                        LargeChange: usize,
                        SetLargeChange: usize,
                        pub Value: unsafe extern "system" fn(
                            *mut core::ffi::c_void,
                            *mut f64,
                        )
                            -> windows_core::HRESULT,
                        pub SetValue: unsafe extern "system" fn(
                            *mut core::ffi::c_void,
                            f64,
                        )
                            -> windows_core::HRESULT,
                    }
                    windows_core::imp::define_interface!(
                        ISelector,
                        ISelector_Vtbl,
                        0x8f7e2159_e61d_576f_8476_f83fde3d689e
                    );
                    impl windows_core::RuntimeType for ISelector {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_interface::<Self>();
                        const NAME: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::from_slice(
                                b"Microsoft.UI.Xaml.Controls.Primitives.ISelector",
                            );
                    }
                    #[repr(C)]
                    pub struct ISelector_Vtbl {
                        pub base__: windows_core::IInspectable_Vtbl,
                        pub SelectedIndex: unsafe extern "system" fn(
                            *mut core::ffi::c_void,
                            *mut i32,
                        )
                            -> windows_core::HRESULT,
                        pub SetSelectedIndex: unsafe extern "system" fn(
                            *mut core::ffi::c_void,
                            i32,
                        )
                            -> windows_core::HRESULT,
                        SelectedItem: usize,
                        SetSelectedItem: usize,
                        SelectedValue: usize,
                        SetSelectedValue: usize,
                        SelectedValuePath: usize,
                        SetSelectedValuePath: usize,
                        IsSynchronizedWithCurrentItem: usize,
                        SetIsSynchronizedWithCurrentItem: usize,
                        pub SelectionChanged: unsafe extern "system" fn(
                            *mut core::ffi::c_void,
                            *mut core::ffi::c_void,
                            *mut i64,
                        )
                            -> windows_core::HRESULT,
                        pub RemoveSelectionChanged:
                            unsafe extern "system" fn(
                                *mut core::ffi::c_void,
                                i64,
                            )
                                -> windows_core::HRESULT,
                    }
                    #[repr(transparent)]
                    #[derive(Clone, Debug, Eq, PartialEq)]
                    pub struct RangeBase(windows_core::IUnknown);
                    windows_core::imp::interface_hierarchy!(
                        RangeBase,
                        windows_core::IUnknown,
                        windows_core::IInspectable
                    );
                    windows_core::imp::required_hierarchy!(
                        RangeBase,
                        super::Control,
                        super::super::FrameworkElement,
                        super::super::UIElement,
                        super::super::DependencyObject
                    );
                    impl RangeBase {
                        pub fn Background(
                            &self,
                        ) -> windows_core::Result<super::super::Media::Brush>
                        {
                            let this = &windows_core::Interface::cast::<super::IControl>(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Background)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .and_then(|| windows_core::imp::Type::from_abi(result__))
                            }
                        }
                        pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                        where
                            P0: windows_core::Param<super::super::Media::Brush>,
                        {
                            let this = &windows_core::Interface::cast::<super::IControl>(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetBackground)(
                                    windows_core::Interface::as_raw(this),
                                    value.param().abi(),
                                )
                                .ok()
                            }
                        }
                        pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).ActualWidth)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).ActualHeight)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn Width(&self) -> windows_core::Result<f64> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Width)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetWidth)(
                                    windows_core::Interface::as_raw(this),
                                    value,
                                )
                                .ok()
                            }
                        }
                        pub fn SizeChanged<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::SizeChangedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            let handler =
                                <super::super::SizeChangedEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemoveSizeChanged,
                                ))
                            }
                        }
                        pub fn FindName(
                            &self,
                            name: &windows_core::HSTRING,
                        ) -> windows_core::Result<windows_core::IInspectable>
                        {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).FindName)(
                                    windows_core::Interface::as_raw(this),
                                    core::mem::transmute_copy(name),
                                    &mut result__,
                                )
                                .and_then(|| windows_core::imp::Type::from_abi(result__))
                            }
                        }
                        pub fn Value(&self) -> windows_core::Result<f64> {
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(self).Value)(
                                    windows_core::Interface::as_raw(self),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn SetValue(&self, value: f64) -> windows_core::Result<()> {
                            unsafe {
                                (windows_core::Interface::vtable(self).SetValue)(
                                    windows_core::Interface::as_raw(self),
                                    value,
                                )
                                .ok()
                            }
                        }
                        pub fn Visibility(&self) -> windows_core::Result<super::super::Visibility> {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Visibility)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn SetVisibility(
                            &self,
                            value: super::super::Visibility,
                        ) -> windows_core::Result<()> {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetVisibility)(
                                    windows_core::Interface::as_raw(this),
                                    value,
                                )
                                .ok()
                            }
                        }
                        pub fn PointerMoved<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::Input::PointerRoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler =
                                <super::super::Input::PointerEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemovePointerMoved,
                                ))
                            }
                        }
                        pub fn PointerExited<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::Input::PointerRoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler =
                                <super::super::Input::PointerEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ =
                                    (windows_core::Interface::vtable(this).PointerExited)(
                                        windows_core::Interface::as_raw(this),
                                        windows_core::Interface::as_raw(&handler),
                                        &mut result__,
                                    )
                                    .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemovePointerExited,
                                ))
                            }
                        }
                        pub fn Tapped<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::Input::TappedRoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler =
                                <super::super::Input::TappedEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemoveTapped,
                                ))
                            }
                        }
                        pub fn DoubleTapped<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<
                                        super::super::Input::DoubleTappedRoutedEventArgs,
                                    >,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler = <super::super::Input::DoubleTappedEventHandler>::new(
                                move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                },
                            );
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemoveDoubleTapped,
                                ))
                            }
                        }


                    }
                    impl windows_core::RuntimeType for RangeBase {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_class::<Self, IRangeBase>();
                    }
                    unsafe impl windows_core::Interface for RangeBase {
                        type Vtable = <IRangeBase as windows_core::Interface>::Vtable;
                        const IID: windows_core::GUID =
                            <IRangeBase as windows_core::Interface>::IID;
                    }
                    impl windows_core::RuntimeName for RangeBase {
                        const NAME: &'static str =
                            "Microsoft.UI.Xaml.Controls.Primitives.RangeBase";
                    }
                    unsafe impl Send for RangeBase {}
                    unsafe impl Sync for RangeBase {}
                    #[repr(transparent)]
                    #[derive(Clone, Debug, Eq, PartialEq)]
                    pub struct Selector(windows_core::IUnknown);
                    windows_core::imp::interface_hierarchy!(
                        Selector,
                        windows_core::IUnknown,
                        windows_core::IInspectable
                    );
                    windows_core::imp::required_hierarchy!(
                        Selector,
                        super::ItemsControl,
                        super::Control,
                        super::super::FrameworkElement,
                        super::super::UIElement,
                        super::super::DependencyObject
                    );
                    impl Selector {
                        pub fn Background(
                            &self,
                        ) -> windows_core::Result<super::super::Media::Brush>
                        {
                            let this = &windows_core::Interface::cast::<super::IControl>(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Background)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .and_then(|| windows_core::imp::Type::from_abi(result__))
                            }
                        }
                        pub fn SetBackground<P0>(&self, value: P0) -> windows_core::Result<()>
                        where
                            P0: windows_core::Param<super::super::Media::Brush>,
                        {
                            let this = &windows_core::Interface::cast::<super::IControl>(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetBackground)(
                                    windows_core::Interface::as_raw(this),
                                    value.param().abi(),
                                )
                                .ok()
                            }
                        }
                        pub fn ActualWidth(&self) -> windows_core::Result<f64> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).ActualWidth)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn ActualHeight(&self) -> windows_core::Result<f64> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).ActualHeight)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn Width(&self) -> windows_core::Result<f64> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Width)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn SetWidth(&self, value: f64) -> windows_core::Result<()> {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetWidth)(
                                    windows_core::Interface::as_raw(this),
                                    value,
                                )
                                .ok()
                            }
                        }
                        pub fn SizeChanged<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::SizeChangedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            let handler =
                                <super::super::SizeChangedEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).SizeChanged)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemoveSizeChanged,
                                ))
                            }
                        }
                        pub fn FindName(
                            &self,
                            name: &windows_core::HSTRING,
                        ) -> windows_core::Result<windows_core::IInspectable>
                        {
                            let this = &windows_core::Interface::cast::<
                                super::super::IFrameworkElement,
                            >(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).FindName)(
                                    windows_core::Interface::as_raw(this),
                                    core::mem::transmute_copy(name),
                                    &mut result__,
                                )
                                .and_then(|| windows_core::imp::Type::from_abi(result__))
                            }
                        }
                        pub fn SelectedIndex(&self) -> windows_core::Result<i32> {
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(self).SelectedIndex)(
                                    windows_core::Interface::as_raw(self),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn SetSelectedIndex(&self, value: i32) -> windows_core::Result<()> {
                            unsafe {
                                (windows_core::Interface::vtable(self).SetSelectedIndex)(
                                    windows_core::Interface::as_raw(self),
                                    value,
                                )
                                .ok()
                            }
                        }
                        pub fn SelectionChanged<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::SelectionChangedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let handler =
                                <super::SelectionChangedEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(self)
                                    .SelectionChanged)(
                                    windows_core::Interface::as_raw(self),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    self.clone(),
                                    token__,
                                    windows_core::Interface::vtable(self).RemoveSelectionChanged,
                                ))
                            }
                        }
                        pub fn Visibility(&self) -> windows_core::Result<super::super::Visibility> {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(this).Visibility)(
                                    windows_core::Interface::as_raw(this),
                                    &mut result__,
                                )
                                .map(|| result__)
                            }
                        }
                        pub fn SetVisibility(
                            &self,
                            value: super::super::Visibility,
                        ) -> windows_core::Result<()> {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            unsafe {
                                (windows_core::Interface::vtable(this).SetVisibility)(
                                    windows_core::Interface::as_raw(this),
                                    value,
                                )
                                .ok()
                            }
                        }
                        pub fn PointerMoved<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::Input::PointerRoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler =
                                <super::super::Input::PointerEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).PointerMoved)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemovePointerMoved,
                                ))
                            }
                        }
                        pub fn PointerExited<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::Input::PointerRoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler =
                                <super::super::Input::PointerEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ =
                                    (windows_core::Interface::vtable(this).PointerExited)(
                                        windows_core::Interface::as_raw(this),
                                        windows_core::Interface::as_raw(&handler),
                                        &mut result__,
                                    )
                                    .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemovePointerExited,
                                ))
                            }
                        }
                        pub fn Tapped<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<super::super::Input::TappedRoutedEventArgs>,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler =
                                <super::super::Input::TappedEventHandler>::new(move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                });
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).Tapped)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemoveTapped,
                                ))
                            }
                        }
                        pub fn DoubleTapped<F>(
                            &self,
                            handler: F,
                        ) -> windows_core::Result<windows_core::EventRevoker>
                        where
                            F: Fn(
                                    windows_core::Ref<windows_core::IInspectable>,
                                    windows_core::Ref<
                                        super::super::Input::DoubleTappedRoutedEventArgs,
                                    >,
                                ) + Send
                                + 'static,
                        {
                            let this =
                                &windows_core::Interface::cast::<super::super::IUIElement>(self)?;
                            let handler = <super::super::Input::DoubleTappedEventHandler>::new(
                                move |a0, a1| {
                                    handler(a0, a1);
                                    Ok(())
                                },
                            );
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                let token__ = (windows_core::Interface::vtable(this).DoubleTapped)(
                                    windows_core::Interface::as_raw(this),
                                    windows_core::Interface::as_raw(&handler),
                                    &mut result__,
                                )
                                .map(|| result__)?;
                                Ok(windows_core::EventRevoker::new(
                                    this.clone(),
                                    token__,
                                    windows_core::Interface::vtable(this).RemoveDoubleTapped,
                                ))
                            }
                        }

                    }
                    impl windows_core::RuntimeType for Selector {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_class::<Self, ISelector>();
                    }
                    unsafe impl windows_core::Interface for Selector {
                        type Vtable = <ISelector as windows_core::Interface>::Vtable;
                        const IID: windows_core::GUID = <ISelector as windows_core::Interface>::IID;
                    }
                    impl windows_core::RuntimeName for Selector {
                        const NAME: &'static str = "Microsoft.UI.Xaml.Controls.Primitives.Selector";
                    }
                    unsafe impl Send for Selector {}
                    unsafe impl Sync for Selector {}
                }
            }
            pub mod Input {
                windows_core::imp::define_interface!(
                    DoubleTappedEventHandler,
                    DoubleTappedEventHandler_Vtbl,
                    0xf7a501b9_e277_5611_87b0_0e0607622183
                );
                impl windows_core::RuntimeType for DoubleTappedEventHandler {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                }
                impl DoubleTappedEventHandler {
                    pub fn new<
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<DoubleTappedRoutedEventArgs>,
                            ) -> windows_core::Result<()>
                            + Send
                            + 'static,
                    >(
                        invoke: F,
                    ) -> Self {
                        let com = windows_core::imp::DelegateBox::<Self, F>::new(
                            &DoubleTappedEventHandlerBox::<F>::VTABLE,
                            invoke,
                        );
                        unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
                    }
                    pub fn Invoke<P0, P1>(&self, sender: P0, e: P1) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<windows_core::IInspectable>,
                        P1: windows_core::Param<DoubleTappedRoutedEventArgs>,
                    {
                        unsafe {
                            (windows_core::Interface::vtable(self).Invoke)(
                                windows_core::Interface::as_raw(self),
                                sender.param().abi(),
                                e.param().abi(),
                            )
                            .ok()
                        }
                    }
                }
                #[repr(C)]
                pub struct DoubleTappedEventHandler_Vtbl {
                    base__: windows_core::IUnknown_Vtbl,
                    Invoke: unsafe extern "system" fn(
                        this: *mut core::ffi::c_void,
                        sender: *mut core::ffi::c_void,
                        e: *mut core::ffi::c_void,
                    ) -> windows_core::HRESULT,
                }
                struct DoubleTappedEventHandlerBox<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<DoubleTappedRoutedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                >(core::marker::PhantomData<(fn() -> F,)>);
                impl<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<DoubleTappedRoutedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                > DoubleTappedEventHandlerBox<F>
                {
                    const VTABLE: DoubleTappedEventHandler_Vtbl =
                        DoubleTappedEventHandler_Vtbl {
                            base__:
                                windows_core::IUnknown_Vtbl {
                                    QueryInterface: windows_core::imp::DelegateBox::<
                                        DoubleTappedEventHandler,
                                        F,
                                    >::QueryInterface,
                                    AddRef: windows_core::imp::DelegateBox::<
                                        DoubleTappedEventHandler,
                                        F,
                                    >::AddRef,
                                    Release: windows_core::imp::DelegateBox::<
                                        DoubleTappedEventHandler,
                                        F,
                                    >::Release,
                                },
                            Invoke: Self::Invoke,
                        };
                    unsafe extern "system" fn Invoke(
                        this: *mut core::ffi::c_void,
                        sender: *mut core::ffi::c_void,
                        e: *mut core::ffi::c_void,
                    ) -> windows_core::HRESULT {
                        unsafe {
                            let this = &mut *(this as *mut *mut core::ffi::c_void
                                as *mut windows_core::imp::DelegateBox<
                                    DoubleTappedEventHandler,
                                    F,
                                >);
                            (this.invoke)(
                                core::mem::transmute_copy(&sender),
                                core::mem::transmute_copy(&e),
                            )
                            .into()
                        }
                    }
                }
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct DoubleTappedRoutedEventArgs(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    DoubleTappedRoutedEventArgs,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    DoubleTappedRoutedEventArgs,
                    super::RoutedEventArgs
                );
                impl DoubleTappedRoutedEventArgs {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            DoubleTappedRoutedEventArgs,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn GetPosition<P0>(
                        &self,
                        relativeto: P0,
                    ) -> windows_core::Result<super::super::super::super::Windows::Foundation::Point>
                    where
                        P0: windows_core::Param<super::UIElement>,
                    {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).GetPosition)(
                                windows_core::Interface::as_raw(self),
                                relativeto.param().abi(),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                }
                impl windows_core::RuntimeType for DoubleTappedRoutedEventArgs {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<
                            Self,
                            IDoubleTappedRoutedEventArgs,
                        >();
                }
                unsafe impl windows_core::Interface for DoubleTappedRoutedEventArgs {
                    type Vtable = <IDoubleTappedRoutedEventArgs as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <IDoubleTappedRoutedEventArgs as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for DoubleTappedRoutedEventArgs {
                    const NAME: &'static str =
                        "Microsoft.UI.Xaml.Input.DoubleTappedRoutedEventArgs";
                }
                unsafe impl Send for DoubleTappedRoutedEventArgs {}
                unsafe impl Sync for DoubleTappedRoutedEventArgs {}
                windows_core::imp::define_interface!(
                    IDoubleTappedRoutedEventArgs,
                    IDoubleTappedRoutedEventArgs_Vtbl,
                    0x32b9549d_11d8_53a5_a953_02409537a11f
                );
                impl windows_core::RuntimeType for IDoubleTappedRoutedEventArgs {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Input.IDoubleTappedRoutedEventArgs",
                        );
                }
                #[repr(C)]
                pub struct IDoubleTappedRoutedEventArgs_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    PointerDeviceType: usize,
                    Handled: usize,
                    SetHandled: usize,
                    pub GetPosition: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut super::super::super::super::Windows::Foundation::Point,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IPointerRoutedEventArgs,
                    IPointerRoutedEventArgs_Vtbl,
                    0x66e78a9a_1bec_5f92_b1a1_ea6334ee511c
                );
                impl windows_core::RuntimeType for IPointerRoutedEventArgs {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Input.IPointerRoutedEventArgs",
                        );
                }
                #[repr(C)]
                pub struct IPointerRoutedEventArgs_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    Pointer: usize,
                    KeyModifiers: usize,
                    Handled: usize,
                    SetHandled: usize,
                    IsGenerated: usize,
                    pub GetCurrentPoint: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    ITappedRoutedEventArgs,
                    ITappedRoutedEventArgs_Vtbl,
                    0x73f74b8c_3709_547e_8e0c_51c03c89126a
                );
                impl windows_core::RuntimeType for ITappedRoutedEventArgs {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Input.ITappedRoutedEventArgs",
                        );
                }
                #[repr(C)]
                pub struct ITappedRoutedEventArgs_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    PointerDeviceType: usize,
                    Handled: usize,
                    SetHandled: usize,
                    pub GetPosition: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut super::super::super::super::Windows::Foundation::Point,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    PointerEventHandler,
                    PointerEventHandler_Vtbl,
                    0xa48a71e1_8bb4_5597_9e31_903a3f6a04fb
                );
                impl windows_core::RuntimeType for PointerEventHandler {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                }
                impl PointerEventHandler {
                    pub fn new<
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<PointerRoutedEventArgs>,
                            ) -> windows_core::Result<()>
                            + Send
                            + 'static,
                    >(
                        invoke: F,
                    ) -> Self {
                        let com = windows_core::imp::DelegateBox::<Self, F>::new(
                            &PointerEventHandlerBox::<F>::VTABLE,
                            invoke,
                        );
                        unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
                    }
                    pub fn Invoke<P0, P1>(&self, sender: P0, e: P1) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<windows_core::IInspectable>,
                        P1: windows_core::Param<PointerRoutedEventArgs>,
                    {
                        unsafe {
                            (windows_core::Interface::vtable(self).Invoke)(
                                windows_core::Interface::as_raw(self),
                                sender.param().abi(),
                                e.param().abi(),
                            )
                            .ok()
                        }
                    }
                }
                #[repr(C)]
                pub struct PointerEventHandler_Vtbl {
                    base__: windows_core::IUnknown_Vtbl,
                    Invoke: unsafe extern "system" fn(
                        this: *mut core::ffi::c_void,
                        sender: *mut core::ffi::c_void,
                        e: *mut core::ffi::c_void,
                    ) -> windows_core::HRESULT,
                }
                struct PointerEventHandlerBox<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<PointerRoutedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                >(core::marker::PhantomData<(fn() -> F,)>);
                impl<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<PointerRoutedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                > PointerEventHandlerBox<F>
                {
                    const VTABLE : PointerEventHandler_Vtbl < > = PointerEventHandler_Vtbl::< > { base__ : windows_core::IUnknown_Vtbl { QueryInterface : windows_core::imp::DelegateBox::< PointerEventHandler , F >::QueryInterface , AddRef : windows_core::imp::DelegateBox::< PointerEventHandler , F >::AddRef , Release : windows_core::imp::DelegateBox::< PointerEventHandler , F >::Release , } , Invoke : Self::Invoke , } ;
                    unsafe extern "system" fn Invoke(
                        this: *mut core::ffi::c_void,
                        sender: *mut core::ffi::c_void,
                        e: *mut core::ffi::c_void,
                    ) -> windows_core::HRESULT {
                        unsafe {
                            let this = &mut *(this as *mut *mut core::ffi::c_void
                                as *mut windows_core::imp::DelegateBox<PointerEventHandler, F>);
                            (this.invoke)(
                                core::mem::transmute_copy(&sender),
                                core::mem::transmute_copy(&e),
                            )
                            .into()
                        }
                    }
                }
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct PointerRoutedEventArgs(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    PointerRoutedEventArgs,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    PointerRoutedEventArgs,
                    super::RoutedEventArgs
                );
                impl PointerRoutedEventArgs {
                    pub fn GetCurrentPoint<P0>(
                        &self,
                        relativeto: P0,
                    ) -> windows_core::Result<super::super::Input::PointerPoint>
                    where
                        P0: windows_core::Param<super::UIElement>,
                    {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).GetCurrentPoint)(
                                windows_core::Interface::as_raw(self),
                                relativeto.param().abi(),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                }
                impl windows_core::RuntimeType for PointerRoutedEventArgs {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IPointerRoutedEventArgs>(
                        );
                }
                unsafe impl windows_core::Interface for PointerRoutedEventArgs {
                    type Vtable = <IPointerRoutedEventArgs as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <IPointerRoutedEventArgs as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for PointerRoutedEventArgs {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Input.PointerRoutedEventArgs";
                }
                unsafe impl Send for PointerRoutedEventArgs {}
                unsafe impl Sync for PointerRoutedEventArgs {}
                windows_core::imp::define_interface!(
                    TappedEventHandler,
                    TappedEventHandler_Vtbl,
                    0xb60074f3_125b_534e_8f9c_9769bd3f0f64
                );
                impl windows_core::RuntimeType for TappedEventHandler {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                }
                impl TappedEventHandler {
                    pub fn new<
                        F: Fn(
                                windows_core::Ref<windows_core::IInspectable>,
                                windows_core::Ref<TappedRoutedEventArgs>,
                            ) -> windows_core::Result<()>
                            + Send
                            + 'static,
                    >(
                        invoke: F,
                    ) -> Self {
                        let com = windows_core::imp::DelegateBox::<Self, F>::new(
                            &TappedEventHandlerBox::<F>::VTABLE,
                            invoke,
                        );
                        unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
                    }
                    pub fn Invoke<P0, P1>(&self, sender: P0, e: P1) -> windows_core::Result<()>
                    where
                        P0: windows_core::Param<windows_core::IInspectable>,
                        P1: windows_core::Param<TappedRoutedEventArgs>,
                    {
                        unsafe {
                            (windows_core::Interface::vtable(self).Invoke)(
                                windows_core::Interface::as_raw(self),
                                sender.param().abi(),
                                e.param().abi(),
                            )
                            .ok()
                        }
                    }
                }
                #[repr(C)]
                pub struct TappedEventHandler_Vtbl {
                    base__: windows_core::IUnknown_Vtbl,
                    Invoke: unsafe extern "system" fn(
                        this: *mut core::ffi::c_void,
                        sender: *mut core::ffi::c_void,
                        e: *mut core::ffi::c_void,
                    ) -> windows_core::HRESULT,
                }
                struct TappedEventHandlerBox<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<TappedRoutedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                >(core::marker::PhantomData<(fn() -> F,)>);
                impl<
                    F: Fn(
                            windows_core::Ref<windows_core::IInspectable>,
                            windows_core::Ref<TappedRoutedEventArgs>,
                        ) -> windows_core::Result<()>
                        + Send
                        + 'static,
                > TappedEventHandlerBox<F>
                {
                    const VTABLE: TappedEventHandler_Vtbl = TappedEventHandler_Vtbl {
                        base__:
                            windows_core::IUnknown_Vtbl {
                                QueryInterface: windows_core::imp::DelegateBox::<
                                    TappedEventHandler,
                                    F,
                                >::QueryInterface,
                                AddRef:
                                    windows_core::imp::DelegateBox::<TappedEventHandler, F>::AddRef,
                                Release:
                                    windows_core::imp::DelegateBox::<TappedEventHandler, F>::Release,
                            },
                        Invoke: Self::Invoke,
                    };
                    unsafe extern "system" fn Invoke(
                        this: *mut core::ffi::c_void,
                        sender: *mut core::ffi::c_void,
                        e: *mut core::ffi::c_void,
                    ) -> windows_core::HRESULT {
                        unsafe {
                            let this = &mut *(this as *mut *mut core::ffi::c_void
                                as *mut windows_core::imp::DelegateBox<TappedEventHandler, F>);
                            (this.invoke)(
                                core::mem::transmute_copy(&sender),
                                core::mem::transmute_copy(&e),
                            )
                            .into()
                        }
                    }
                }
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct TappedRoutedEventArgs(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    TappedRoutedEventArgs,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    TappedRoutedEventArgs,
                    super::RoutedEventArgs
                );
                impl TappedRoutedEventArgs {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            TappedRoutedEventArgs,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn GetPosition<P0>(
                        &self,
                        relativeto: P0,
                    ) -> windows_core::Result<super::super::super::super::Windows::Foundation::Point>
                    where
                        P0: windows_core::Param<super::UIElement>,
                    {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).GetPosition)(
                                windows_core::Interface::as_raw(self),
                                relativeto.param().abi(),
                                &mut result__,
                            )
                            .map(|| result__)
                        }
                    }
                }
                impl windows_core::RuntimeType for TappedRoutedEventArgs {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, ITappedRoutedEventArgs>();
                }
                unsafe impl windows_core::Interface for TappedRoutedEventArgs {
                    type Vtable = <ITappedRoutedEventArgs as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <ITappedRoutedEventArgs as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for TappedRoutedEventArgs {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Input.TappedRoutedEventArgs";
                }
                unsafe impl Send for TappedRoutedEventArgs {}
                unsafe impl Sync for TappedRoutedEventArgs {}
            }
            pub mod Markup {
                windows_core::imp::define_interface!(
                    IXamlMetadataProvider,
                    IXamlMetadataProvider_Vtbl,
                    0xa96251f0_2214_5d53_8746_ce99a2593cd7
                );
                impl windows_core::RuntimeType for IXamlMetadataProvider {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Markup.IXamlMetadataProvider",
                        );
                }
                windows_core::imp::interface_hierarchy!(
                    IXamlMetadataProvider,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                impl IXamlMetadataProvider {
                    pub fn GetXamlType(
                        &self,
                        r#type: &super::super::super::super::Windows::UI::Xaml::Interop::TypeName,
                    ) -> windows_core::Result<IXamlType> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).GetXamlType)(
                                windows_core::Interface::as_raw(self),
                                core::mem::transmute_copy(r#type),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn GetXamlTypeByFullName(
                        &self,
                        fullname: &windows_core::HSTRING,
                    ) -> windows_core::Result<IXamlType> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).GetXamlTypeByFullName)(
                                windows_core::Interface::as_raw(self),
                                core::mem::transmute_copy(fullname),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn GetXmlnsDefinitions(
                        &self,
                    ) -> windows_core::Result<windows_core::Array<XmlnsDefinition>>
                    {
                        unsafe {
                            let mut result__ = core::mem::MaybeUninit::zeroed();
                            (windows_core::Interface::vtable(self).GetXmlnsDefinitions)(
                                windows_core::Interface::as_raw(self),
                                windows_core::Array::<XmlnsDefinition>::set_abi_len(
                                    core::mem::transmute(&mut result__),
                                ),
                                result__.as_mut_ptr() as *mut _ as _,
                            )
                            .map(|| result__.assume_init())
                        }
                    }
                }
                impl windows_core::RuntimeName for IXamlMetadataProvider {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Markup.IXamlMetadataProvider";
                }
                pub trait IXamlMetadataProvider_Impl: windows_core::IUnknownImpl {
                    fn GetXamlType(
                        &self,
                        r#type: &super::super::super::super::Windows::UI::Xaml::Interop::TypeName,
                    ) -> windows_core::Result<IXamlType>;
                    fn GetXamlTypeByFullName(
                        &self,
                        fullName: &windows_core::HSTRING,
                    ) -> windows_core::Result<IXamlType>;
                    fn GetXmlnsDefinitions(
                        &self,
                    ) -> windows_core::Result<windows_core::Array<XmlnsDefinition>>;
                }
                impl IXamlMetadataProvider_Vtbl {
                    pub const fn new<Identity: IXamlMetadataProvider_Impl, const OFFSET: isize>()
                    -> Self {
                        unsafe extern "system" fn GetXamlType<
                            Identity: IXamlMetadataProvider_Impl,
                            const OFFSET: isize,
                        >(
                            this: *mut core::ffi::c_void,
                            r#type: core::mem::MaybeUninit<
                                super::super::super::super::Windows::UI::Xaml::Interop::TypeName,
                            >,
                            result__: *mut *mut core::ffi::c_void,
                        ) -> windows_core::HRESULT {
                            unsafe {
                                let this: &Identity = &*((this as *const *const ()).offset(OFFSET)
                                    as *const Identity);
                                match IXamlMetadataProvider_Impl::GetXamlType(
                                    this,
                                    core::mem::transmute(&r#type),
                                ) {
                                    Ok(ok__) => {
                                        result__.write(core::mem::transmute_copy(&ok__));
                                        core::mem::forget(ok__);
                                        windows_core::HRESULT(0)
                                    }
                                    Err(err) => err.into(),
                                }
                            }
                        }
                        unsafe extern "system" fn GetXamlTypeByFullName<
                            Identity: IXamlMetadataProvider_Impl,
                            const OFFSET: isize,
                        >(
                            this: *mut core::ffi::c_void,
                            fullname: *mut core::ffi::c_void,
                            result__: *mut *mut core::ffi::c_void,
                        ) -> windows_core::HRESULT {
                            unsafe {
                                let this: &Identity = &*((this as *const *const ()).offset(OFFSET)
                                    as *const Identity);
                                match IXamlMetadataProvider_Impl::GetXamlTypeByFullName(
                                    this,
                                    core::mem::transmute(&fullname),
                                ) {
                                    Ok(ok__) => {
                                        result__.write(core::mem::transmute_copy(&ok__));
                                        core::mem::forget(ok__);
                                        windows_core::HRESULT(0)
                                    }
                                    Err(err) => err.into(),
                                }
                            }
                        }
                        unsafe extern "system" fn GetXmlnsDefinitions<
                            Identity: IXamlMetadataProvider_Impl,
                            const OFFSET: isize,
                        >(
                            this: *mut core::ffi::c_void,
                            result_size__: *mut u32,
                            result__: *mut *mut core::mem::MaybeUninit<XmlnsDefinition>,
                        ) -> windows_core::HRESULT {
                            unsafe {
                                let this: &Identity = &*((this as *const *const ()).offset(OFFSET)
                                    as *const Identity);
                                match IXamlMetadataProvider_Impl::GetXmlnsDefinitions(this) {
                                    Ok(ok__) => {
                                        let (ok_data__, ok_data_len__) = ok__.into_abi();
                                        result__.write(ok_data__);
                                        result_size__.write(ok_data_len__);
                                        windows_core::HRESULT(0)
                                    }
                                    Err(err) => err.into(),
                                }
                            }
                        }
                        Self {
                            base__: windows_core::IInspectable_Vtbl::new::<
                                Identity,
                                IXamlMetadataProvider,
                                OFFSET,
                            >(),
                            GetXamlType: GetXamlType::<Identity, OFFSET>,
                            GetXamlTypeByFullName: GetXamlTypeByFullName::<Identity, OFFSET>,
                            GetXmlnsDefinitions: GetXmlnsDefinitions::<Identity, OFFSET>,
                        }
                    }
                    pub fn matches(iid: &windows_core::GUID) -> bool {
                        iid == &<IXamlMetadataProvider as windows_core::Interface>::IID
                    }
                }
                #[repr(C)]
                pub struct IXamlMetadataProvider_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    pub GetXamlType: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        core::mem::MaybeUninit<
                            super::super::super::super::Windows::UI::Xaml::Interop::TypeName,
                        >,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                    pub GetXamlTypeByFullName: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                    pub GetXmlnsDefinitions: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut u32,
                        *mut *mut core::mem::MaybeUninit<XmlnsDefinition>,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IXamlReader,
                    IXamlReader_Vtbl,
                    0x54ce54c8_38c6_50d9_ac98_4b03eddbde9f
                );
                impl windows_core::RuntimeType for IXamlReader {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Markup.IXamlReader",
                        );
                }
                #[repr(C)]
                pub struct IXamlReader_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                windows_core::imp::define_interface!(
                    IXamlReaderStatics,
                    IXamlReaderStatics_Vtbl,
                    0x82a4cd9e_435e_5aeb_8c4f_300cece45cae
                );
                impl windows_core::RuntimeType for IXamlReaderStatics {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Markup.IXamlReaderStatics",
                        );
                }
                #[repr(C)]
                pub struct IXamlReaderStatics_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    pub Load: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    IXamlType,
                    IXamlType_Vtbl,
                    0xd24219df_7ec9_57f1_a27b_6af251d9c5bc
                );
                impl windows_core::RuntimeType for IXamlType {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Markup.IXamlType",
                        );
                }
                windows_core::imp::interface_hierarchy!(
                    IXamlType,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                #[repr(C)]
                pub struct IXamlType_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct XamlReader(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    XamlReader,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                impl XamlReader {
                    pub fn Load(
                        xaml: &windows_core::HSTRING,
                    ) -> windows_core::Result<windows_core::IInspectable> {
                        Self::IXamlReaderStatics(|this| unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).Load)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(xaml),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        })
                    }
                    fn IXamlReaderStatics<
                        R,
                        F: FnOnce(&IXamlReaderStatics) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            XamlReader,
                            IXamlReaderStatics,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                }
                impl windows_core::RuntimeType for XamlReader {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IXamlReader>();
                }
                unsafe impl windows_core::Interface for XamlReader {
                    type Vtable = <IXamlReader as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IXamlReader as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for XamlReader {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Markup.XamlReader";
                }
                unsafe impl Send for XamlReader {}
                unsafe impl Sync for XamlReader {}
                #[repr(C)]
                #[derive(Clone, Debug, Default, Eq, PartialEq)]
                pub struct XmlnsDefinition {
                    pub XmlNamespace: windows_core::HSTRING,
                    pub Namespace: windows_core::HSTRING,
                }
                impl windows_core::imp::TypeKind for XmlnsDefinition {
                    type TypeKind = windows_core::imp::CloneType;
                }
                impl windows_core::RuntimeType for XmlnsDefinition {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"struct(Microsoft.UI.Xaml.Markup.XmlnsDefinition;string;string)",
                        );
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Markup.XmlnsDefinition",
                        );
                }
            }
            pub mod Media {
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct Brush(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    Brush,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(Brush, super::DependencyObject);
                impl Brush {


                }
                impl windows_core::RuntimeType for Brush {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IBrush>();
                }
                unsafe impl windows_core::Interface for Brush {
                    type Vtable = <IBrush as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IBrush as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for Brush {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Media.Brush";
                }
                unsafe impl Send for Brush {}
                unsafe impl Sync for Brush {}
                windows_core::imp::define_interface!(
                    IBrush,
                    IBrush_Vtbl,
                    0x2de3cb83_1329_5679_88f8_c822bc5442cb
                );
                impl windows_core::RuntimeType for IBrush {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Media.IBrush",
                        );
                }
                #[repr(C)]
                pub struct IBrush_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                windows_core::imp::define_interface!(
                    IImageSource,
                    IImageSource_Vtbl,
                    0x6c2038f6_d6d5_55e9_9b9e_082f12dbff60
                );
                impl windows_core::RuntimeType for IImageSource {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Media.IImageSource",
                        );
                }
                #[repr(C)]
                pub struct IImageSource_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                windows_core::imp::define_interface!(
                    IMicaBackdrop,
                    IMicaBackdrop_Vtbl,
                    0xc156a404_3dac_593a_b1f3_7a33c289dc83
                );
                impl windows_core::RuntimeType for IMicaBackdrop {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Media.IMicaBackdrop",
                        );
                }
                #[repr(C)]
                pub struct IMicaBackdrop_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                windows_core::imp::define_interface!(
                    IMicaBackdropFactory,
                    IMicaBackdropFactory_Vtbl,
                    0x774379ce_74bd_59d4_849d_d99c4184d838
                );
                impl windows_core::RuntimeType for IMicaBackdropFactory {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Media.IMicaBackdropFactory",
                        );
                }
                #[repr(C)]
                pub struct IMicaBackdropFactory_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                    pub CreateInstance: unsafe extern "system" fn(
                        *mut core::ffi::c_void,
                        *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                        *mut *mut core::ffi::c_void,
                    )
                        -> windows_core::HRESULT,
                }
                windows_core::imp::define_interface!(
                    ISystemBackdrop,
                    ISystemBackdrop_Vtbl,
                    0x5aeed5c4_37ac_5852_b73f_1b76ebc3205f
                );
                impl windows_core::RuntimeType for ISystemBackdrop {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_interface::<Self>();
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Microsoft.UI.Xaml.Media.ISystemBackdrop",
                        );
                }
                #[repr(C)]
                pub struct ISystemBackdrop_Vtbl {
                    pub base__: windows_core::IInspectable_Vtbl,
                }
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct ImageSource(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    ImageSource,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(ImageSource, super::DependencyObject);
                impl windows_core::RuntimeType for ImageSource {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IImageSource>();
                }
                unsafe impl windows_core::Interface for ImageSource {
                    type Vtable = <IImageSource as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IImageSource as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for ImageSource {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Media.ImageSource";
                }
                unsafe impl Send for ImageSource {}
                unsafe impl Sync for ImageSource {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct MicaBackdrop(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    MicaBackdrop,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(
                    MicaBackdrop,
                    SystemBackdrop,
                    super::DependencyObject
                );
                impl MicaBackdrop {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IMicaBackdropFactory(|this| unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).CreateInstance)(
                                windows_core::Interface::as_raw(this),
                                core::ptr::null_mut(),
                                core::ptr::null_mut(),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        })
                    }
                    pub fn compose<T>(compose: T) -> windows_core::Result<Self>
                    where
                        T: windows_core::Compose,
                    {
                        Self::IMicaBackdropFactory(|this| unsafe {
                            let (derived__, base__) = windows_core::Compose::compose(compose);
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(this).CreateInstance)(
                                windows_core::Interface::as_raw(this),
                                core::mem::transmute_copy(&derived__),
                                base__ as *mut _ as _,
                                &mut result__,
                            )
                            .ok()?;
                            let _ = &derived__;
                            windows_core::imp::Type::from_abi(result__)
                        })
                    }
                    fn IMicaBackdropFactory<
                        R,
                        F: FnOnce(&IMicaBackdropFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            MicaBackdrop,
                            IMicaBackdropFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }

                }
                impl windows_core::RuntimeType for MicaBackdrop {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, IMicaBackdrop>();
                }
                unsafe impl windows_core::Interface for MicaBackdrop {
                    type Vtable = <IMicaBackdrop as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID = <IMicaBackdrop as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for MicaBackdrop {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Media.MicaBackdrop";
                }
                unsafe impl Send for MicaBackdrop {}
                unsafe impl Sync for MicaBackdrop {}
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct SystemBackdrop(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    SystemBackdrop,
                    windows_core::IUnknown,
                    windows_core::IInspectable
                );
                windows_core::imp::required_hierarchy!(SystemBackdrop, super::DependencyObject);
                impl SystemBackdrop {

                }
                impl windows_core::RuntimeType for SystemBackdrop {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<Self, ISystemBackdrop>();
                }
                unsafe impl windows_core::Interface for SystemBackdrop {
                    type Vtable = <ISystemBackdrop as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <ISystemBackdrop as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for SystemBackdrop {
                    const NAME: &'static str = "Microsoft.UI.Xaml.Media.SystemBackdrop";
                }
                unsafe impl Send for SystemBackdrop {}
                unsafe impl Sync for SystemBackdrop {}
                pub mod Imaging {
                    #[repr(transparent)]
                    #[derive(Clone, Debug, Eq, PartialEq)]
                    pub struct BitmapSource(windows_core::IUnknown);
                    windows_core::imp::interface_hierarchy!(
                        BitmapSource,
                        windows_core::IUnknown,
                        windows_core::IInspectable
                    );
                    windows_core::imp::required_hierarchy!(
                        BitmapSource,
                        super::ImageSource,
                        super::super::DependencyObject
                    );
                    impl BitmapSource {


                    }
                    impl windows_core::RuntimeType for BitmapSource {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_class::<Self, IBitmapSource>();
                    }
                    unsafe impl windows_core::Interface for BitmapSource {
                        type Vtable = <IBitmapSource as windows_core::Interface>::Vtable;
                        const IID: windows_core::GUID =
                            <IBitmapSource as windows_core::Interface>::IID;
                    }
                    impl windows_core::RuntimeName for BitmapSource {
                        const NAME: &'static str = "Microsoft.UI.Xaml.Media.Imaging.BitmapSource";
                    }
                    unsafe impl Send for BitmapSource {}
                    unsafe impl Sync for BitmapSource {}
                    windows_core::imp::define_interface!(
                        IBitmapSource,
                        IBitmapSource_Vtbl,
                        0x8424269d_9b82_534f_8fea_af5b5ef96bf2
                    );
                    impl windows_core::RuntimeType for IBitmapSource {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_interface::<Self>();
                        const NAME: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::from_slice(
                                b"Microsoft.UI.Xaml.Media.Imaging.IBitmapSource",
                            );
                    }
                    #[repr(C)]
                    pub struct IBitmapSource_Vtbl {
                        pub base__: windows_core::IInspectable_Vtbl,
                    }
                    windows_core::imp::define_interface!(
                        IWriteableBitmap,
                        IWriteableBitmap_Vtbl,
                        0x78c824a9_0e43_5f1e_93bc_d046cca82b7e
                    );
                    impl windows_core::RuntimeType for IWriteableBitmap {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_interface::<Self>();
                        const NAME: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::from_slice(
                                b"Microsoft.UI.Xaml.Media.Imaging.IWriteableBitmap",
                            );
                    }
                    #[repr(C)]
                    pub struct IWriteableBitmap_Vtbl {
                        pub base__: windows_core::IInspectable_Vtbl,
                        pub PixelBuffer: unsafe extern "system" fn(
                            *mut core::ffi::c_void,
                            *mut *mut core::ffi::c_void,
                        )
                            -> windows_core::HRESULT,
                        pub Invalidate: unsafe extern "system" fn(
                            *mut core::ffi::c_void,
                        )
                            -> windows_core::HRESULT,
                    }
                    windows_core::imp::define_interface!(
                        IWriteableBitmapFactory,
                        IWriteableBitmapFactory_Vtbl,
                        0x26e861d9_b080_512b_96c4_80050e7e08d1
                    );
                    impl windows_core::RuntimeType for IWriteableBitmapFactory {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_interface::<Self>();
                        const NAME: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::from_slice(
                                b"Microsoft.UI.Xaml.Media.Imaging.IWriteableBitmapFactory",
                            );
                    }
                    #[repr(C)]
                    pub struct IWriteableBitmapFactory_Vtbl {
                        pub base__: windows_core::IInspectable_Vtbl,
                        pub CreateInstanceWithDimensions:
                            unsafe extern "system" fn(
                                *mut core::ffi::c_void,
                                i32,
                                i32,
                                *mut *mut core::ffi::c_void,
                            )
                                -> windows_core::HRESULT,
                    }
                    #[repr(transparent)]
                    #[derive(Clone, Debug, Eq, PartialEq)]
                    pub struct WriteableBitmap(windows_core::IUnknown);
                    windows_core::imp::interface_hierarchy!(
                        WriteableBitmap,
                        windows_core::IUnknown,
                        windows_core::IInspectable
                    );
                    windows_core::imp::required_hierarchy!(
                        WriteableBitmap,
                        BitmapSource,
                        super::ImageSource,
                        super::super::DependencyObject
                    );
                    impl WriteableBitmap {
                        pub fn PixelBuffer(
                            &self,
                        ) -> windows_core::Result<
                            super::super::super::super::super::Windows::Storage::Streams::IBuffer,
                        > {
                            unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable(self).PixelBuffer)(
                                    windows_core::Interface::as_raw(self),
                                    &mut result__,
                                )
                                .and_then(|| windows_core::imp::Type::from_abi(result__))
                            }
                        }
                        pub fn Invalidate(&self) -> windows_core::Result<()> {
                            unsafe {
                                (windows_core::Interface::vtable(self).Invalidate)(
                                    windows_core::Interface::as_raw(self),
                                )
                                .ok()
                            }
                        }
                        pub fn CreateInstanceWithDimensions(
                            pixelwidth: i32,
                            pixelheight: i32,
                        ) -> windows_core::Result<Self> {
                            Self::IWriteableBitmapFactory(|this| unsafe {
                                let mut result__ = core::mem::zeroed();
                                (windows_core::Interface::vtable (this) . CreateInstanceWithDimensions)(windows_core::Interface::as_raw (this) , pixelwidth , pixelheight , & mut result__) . and_then (|| windows_core::imp::Type::from_abi (result__))
                            })
                        }
                        fn IWriteableBitmapFactory<
                            R,
                            F: FnOnce(&IWriteableBitmapFactory) -> windows_core::Result<R>,
                        >(
                            callback: F,
                        ) -> windows_core::Result<R> {
                            static SHARED: windows_core::imp::FactoryCache<
                                WriteableBitmap,
                                IWriteableBitmapFactory,
                            > = windows_core::imp::FactoryCache::new();
                            SHARED.call(callback)
                        }
                    }
                    impl windows_core::RuntimeType for WriteableBitmap {
                        const SIGNATURE: windows_core::imp::ConstBuffer =
                            windows_core::imp::ConstBuffer::for_class::<Self, IWriteableBitmap>();
                    }
                    unsafe impl windows_core::Interface for WriteableBitmap {
                        type Vtable = <IWriteableBitmap as windows_core::Interface>::Vtable;
                        const IID: windows_core::GUID =
                            <IWriteableBitmap as windows_core::Interface>::IID;
                    }
                    impl windows_core::RuntimeName for WriteableBitmap {
                        const NAME: &'static str =
                            "Microsoft.UI.Xaml.Media.Imaging.WriteableBitmap";
                    }
                    unsafe impl Send for WriteableBitmap {}
                    unsafe impl Sync for WriteableBitmap {}
                }
            }
            pub mod XamlTypeInfo {
                #[repr(transparent)]
                #[derive(Clone, Debug, Eq, PartialEq)]
                pub struct XamlControlsXamlMetaDataProvider(windows_core::IUnknown);
                windows_core::imp::interface_hierarchy!(
                    XamlControlsXamlMetaDataProvider,
                    windows_core::IUnknown,
                    windows_core::IInspectable,
                    super::Markup::IXamlMetadataProvider
                );
                impl XamlControlsXamlMetaDataProvider {
                    pub fn new() -> windows_core::Result<Self> {
                        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
                    }
                    fn IActivationFactory<
                        R,
                        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
                    >(
                        callback: F,
                    ) -> windows_core::Result<R> {
                        static SHARED: windows_core::imp::FactoryCache<
                            XamlControlsXamlMetaDataProvider,
                            windows_core::imp::IGenericFactory,
                        > = windows_core::imp::FactoryCache::new();
                        SHARED.call(callback)
                    }
                    pub fn GetXamlType(
                        &self,
                        r#type: &super::super::super::super::Windows::UI::Xaml::Interop::TypeName,
                    ) -> windows_core::Result<super::Markup::IXamlType> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).GetXamlType)(
                                windows_core::Interface::as_raw(self),
                                core::mem::transmute_copy(r#type),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn GetXamlTypeByFullName(
                        &self,
                        fullname: &windows_core::HSTRING,
                    ) -> windows_core::Result<super::Markup::IXamlType> {
                        unsafe {
                            let mut result__ = core::mem::zeroed();
                            (windows_core::Interface::vtable(self).GetXamlTypeByFullName)(
                                windows_core::Interface::as_raw(self),
                                core::mem::transmute_copy(fullname),
                                &mut result__,
                            )
                            .and_then(|| windows_core::imp::Type::from_abi(result__))
                        }
                    }
                    pub fn GetXmlnsDefinitions(
                        &self,
                    ) -> windows_core::Result<windows_core::Array<super::Markup::XmlnsDefinition>>
                    {
                        unsafe {
                            let mut result__ = core::mem::MaybeUninit::zeroed();
                            (windows_core::Interface::vtable(self).GetXmlnsDefinitions)(
                                windows_core::Interface::as_raw(self),
                                windows_core::Array::<super::Markup::XmlnsDefinition>::set_abi_len(
                                    core::mem::transmute(&mut result__),
                                ),
                                result__.as_mut_ptr() as *mut _ as _,
                            )
                            .map(|| result__.assume_init())
                        }
                    }

                }
                impl windows_core::RuntimeType for XamlControlsXamlMetaDataProvider {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::for_class::<
                            Self,
                            super::Markup::IXamlMetadataProvider,
                        >();
                }
                unsafe impl windows_core::Interface for XamlControlsXamlMetaDataProvider {
                    type Vtable =
                        <super::Markup::IXamlMetadataProvider as windows_core::Interface>::Vtable;
                    const IID: windows_core::GUID =
                        <super::Markup::IXamlMetadataProvider as windows_core::Interface>::IID;
                }
                impl windows_core::RuntimeName for XamlControlsXamlMetaDataProvider {
                    const NAME: &'static str =
                        "Microsoft.UI.Xaml.XamlTypeInfo.XamlControlsXamlMetaDataProvider";
                }
                unsafe impl Send for XamlControlsXamlMetaDataProvider {}
                unsafe impl Sync for XamlControlsXamlMetaDataProvider {}
            }
        }
    }
}
pub mod Windows {
    pub mod Foundation {
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        pub struct Point {
            pub X: f32,
            pub Y: f32,
        }
        impl windows_core::imp::TypeKind for Point {
            type TypeKind = windows_core::imp::CopyType;
        }
        impl windows_core::RuntimeType for Point {
            const SIGNATURE: windows_core::imp::ConstBuffer =
                windows_core::imp::ConstBuffer::from_slice(
                    b"struct(Windows.Foundation.Point;f4;f4)",
                );
            const NAME: windows_core::imp::ConstBuffer =
                windows_core::imp::ConstBuffer::from_slice(b"Windows.Foundation.Point");
        }
        #[repr(transparent)]
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct TypedEventHandler<TSender, TResult>(
            windows_core::IUnknown,
            core::marker::PhantomData<TSender>,
            core::marker::PhantomData<TResult>,
        )
        where
            TSender: windows_core::RuntimeType + 'static,
            TResult: windows_core::RuntimeType + 'static;
        unsafe impl<
            TSender: windows_core::RuntimeType + 'static,
            TResult: windows_core::RuntimeType + 'static,
        > windows_core::Interface for TypedEventHandler<TSender, TResult>
        {
            type Vtable = TypedEventHandler_Vtbl<TSender, TResult>;
            const IID: windows_core::GUID =
                windows_core::GUID::from_signature(<Self as windows_core::RuntimeType>::SIGNATURE);
        }
        impl<
            TSender: windows_core::RuntimeType + 'static,
            TResult: windows_core::RuntimeType + 'static,
        > windows_core::RuntimeType for TypedEventHandler<TSender, TResult>
        {
            const SIGNATURE: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::new()
                .push_slice(b"pinterface({9de1c534-6ae1-11e0-84e1-18a905bcc53f}")
                .push_slice(b";")
                .push_other(TSender::SIGNATURE)
                .push_slice(b";")
                .push_other(TResult::SIGNATURE)
                .push_slice(b")");
        }
        impl<
            TSender: windows_core::RuntimeType + 'static,
            TResult: windows_core::RuntimeType + 'static,
        > TypedEventHandler<TSender, TResult>
        {
            pub fn new<
                F: Fn(
                        windows_core::Ref<TSender>,
                        windows_core::Ref<TResult>,
                    ) -> windows_core::Result<()>
                    + Send
                    + 'static,
            >(
                invoke: F,
            ) -> Self {
                let com = windows_core::imp::DelegateBox::<Self, F>::new(
                    &TypedEventHandlerBox::<TSender, TResult, F>::VTABLE,
                    invoke,
                );
                unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
            }
            pub fn Invoke<P0, P1>(&self, sender: P0, args: P1) -> windows_core::Result<()>
            where
                P0: windows_core::Param<TSender>,
                P1: windows_core::Param<TResult>,
            {
                unsafe {
                    (windows_core::Interface::vtable(self).Invoke)(
                        windows_core::Interface::as_raw(self),
                        sender.param().abi(),
                        args.param().abi(),
                    )
                    .ok()
                }
            }
        }
        #[repr(C)]
        pub struct TypedEventHandler_Vtbl<TSender, TResult>
        where
            TSender: windows_core::RuntimeType + 'static,
            TResult: windows_core::RuntimeType + 'static,
        {
            base__: windows_core::IUnknown_Vtbl,
            Invoke: unsafe extern "system" fn(
                this: *mut core::ffi::c_void,
                sender: windows_core::imp::AbiType<TSender>,
                args: windows_core::imp::AbiType<TResult>,
            ) -> windows_core::HRESULT,
            TSender: core::marker::PhantomData<TSender>,
            TResult: core::marker::PhantomData<TResult>,
        }
        struct TypedEventHandlerBox<
            TSender,
            TResult,
            F: Fn(
                    windows_core::Ref<TSender>,
                    windows_core::Ref<TResult>,
                ) -> windows_core::Result<()>
                + Send
                + 'static,
        >(core::marker::PhantomData<(TSender, TResult, fn() -> F)>)
        where
            TSender: windows_core::RuntimeType + 'static,
            TResult: windows_core::RuntimeType + 'static;
        impl<
            TSender: windows_core::RuntimeType + 'static,
            TResult: windows_core::RuntimeType + 'static,
            F: Fn(
                    windows_core::Ref<TSender>,
                    windows_core::Ref<TResult>,
                ) -> windows_core::Result<()>
                + Send
                + 'static,
        > TypedEventHandlerBox<TSender, TResult, F>
        {
            const VTABLE: TypedEventHandler_Vtbl<TSender, TResult> =
                TypedEventHandler_Vtbl::<TSender, TResult> {
                    base__: windows_core::IUnknown_Vtbl {
                        QueryInterface: windows_core::imp::DelegateBox::<
                            TypedEventHandler<TSender, TResult>,
                            F,
                        >::QueryInterface,
                        AddRef: windows_core::imp::DelegateBox::<
                            TypedEventHandler<TSender, TResult>,
                            F,
                        >::AddRef,
                        Release: windows_core::imp::DelegateBox::<
                            TypedEventHandler<TSender, TResult>,
                            F,
                        >::Release,
                    },
                    Invoke: Self::Invoke,
                    TSender: core::marker::PhantomData::<TSender>,
                    TResult: core::marker::PhantomData::<TResult>,
                };
            unsafe extern "system" fn Invoke(
                this: *mut core::ffi::c_void,
                sender: windows_core::imp::AbiType<TSender>,
                args: windows_core::imp::AbiType<TResult>,
            ) -> windows_core::HRESULT {
                unsafe {
                    let this = &mut *(this as *mut *mut core::ffi::c_void
                        as *mut windows_core::imp::DelegateBox<
                            TypedEventHandler<TSender, TResult>,
                            F,
                        >);
                    (this.invoke)(
                        core::mem::transmute_copy(&sender),
                        core::mem::transmute_copy(&args),
                    )
                    .into()
                }
            }
        }
        pub mod Collections {
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct IIterable<T>(windows_core::IUnknown, core::marker::PhantomData<T>)
            where
                T: windows_core::RuntimeType + 'static;
            impl<T: windows_core::RuntimeType + 'static>
                windows_core::imp::CanInto<windows_core::IUnknown> for IIterable<T>
            {
            }
            impl<T: windows_core::RuntimeType + 'static>
                windows_core::imp::CanInto<windows_core::IInspectable> for IIterable<T>
            {
            }
            unsafe impl<T: windows_core::RuntimeType + 'static> windows_core::Interface for IIterable<T> {
                type Vtable = IIterable_Vtbl<T>;
                const IID: windows_core::GUID = windows_core::GUID::from_signature(
                    <Self as windows_core::RuntimeType>::SIGNATURE,
                );
            }
            impl<T: windows_core::RuntimeType + 'static> windows_core::RuntimeType for IIterable<T> {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::new()
                        .push_slice(b"pinterface({faa585ea-6214-4217-afda-7f46de5869b3}")
                        .push_slice(b";")
                        .push_other(T::SIGNATURE)
                        .push_slice(b")");
                const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::new()
                    .push_slice(b"Windows.Foundation.Collections.IIterable`1<")
                    .push_other(T::NAME)
                    .push_slice(b">");
            }
            impl<T: windows_core::RuntimeType + 'static> IIterable<T> {
                pub fn First(&self) -> windows_core::Result<windows_collections::IIterator<T>> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).First)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
            }
            #[repr(C)]
            pub struct IIterable_Vtbl<T>
            where
                T: windows_core::RuntimeType + 'static,
            {
                pub base__: windows_core::IInspectable_Vtbl,
                pub First: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
                T: core::marker::PhantomData<T>,
            }
            impl<T: windows_core::RuntimeType> IntoIterator for IIterable<T> {
                type Item = T;
                type IntoIter = windows_collections::BufferedIterator<Self::Item>;
                fn into_iter(self) -> Self::IntoIter {
                    IntoIterator::into_iter(&self)
                }
            }
            impl<T: windows_core::RuntimeType> IntoIterator for &IIterable<T> {
                type Item = T;
                type IntoIter = windows_collections::BufferedIterator<Self::Item>;
                fn into_iter(self) -> Self::IntoIter {
                    windows_collections::BufferedIterator::new(self.First().unwrap())
                }
            }
            #[repr(transparent)]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct IVector<T>(windows_core::IUnknown, core::marker::PhantomData<T>)
            where
                T: windows_core::RuntimeType + 'static;
            impl<T: windows_core::RuntimeType + 'static>
                windows_core::imp::CanInto<windows_core::IUnknown> for IVector<T>
            {
            }
            impl<T: windows_core::RuntimeType + 'static>
                windows_core::imp::CanInto<windows_core::IInspectable> for IVector<T>
            {
            }
            unsafe impl<T: windows_core::RuntimeType + 'static> windows_core::Interface for IVector<T> {
                type Vtable = IVector_Vtbl<T>;
                const IID: windows_core::GUID = windows_core::GUID::from_signature(
                    <Self as windows_core::RuntimeType>::SIGNATURE,
                );
            }
            impl<T: windows_core::RuntimeType + 'static> windows_core::RuntimeType for IVector<T> {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::new()
                        .push_slice(b"pinterface({913337e9-11a1-4345-a3a2-4e7f956e222d}")
                        .push_slice(b";")
                        .push_other(T::SIGNATURE)
                        .push_slice(b")");
                const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::new()
                    .push_slice(b"Windows.Foundation.Collections.IVector`1<")
                    .push_other(T::NAME)
                    .push_slice(b">");
            }
            impl<T: windows_core::RuntimeType + 'static> windows_core::imp::CanInto<IIterable<T>>
                for IVector<T>
            {
                const QUERY: bool = true;
            }
            impl<T: windows_core::RuntimeType + 'static> IVector<T> {
                pub fn GetAt(&self, index: u32) -> windows_core::Result<T> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).GetAt)(
                            windows_core::Interface::as_raw(self),
                            index,
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
                pub fn Size(&self) -> windows_core::Result<u32> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Size)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn GetView(&self) -> windows_core::Result<windows_collections::IVectorView<T>> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).GetView)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
                pub fn IndexOf<P0>(&self, value: P0, index: &mut u32) -> windows_core::Result<bool>
                where
                    P0: windows_core::Param<T>,
                {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).IndexOf)(
                            windows_core::Interface::as_raw(self),
                            value.param().abi(),
                            index,
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn SetAt<P1>(&self, index: u32, value: P1) -> windows_core::Result<()>
                where
                    P1: windows_core::Param<T>,
                {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetAt)(
                            windows_core::Interface::as_raw(self),
                            index,
                            value.param().abi(),
                        )
                        .ok()
                    }
                }
                pub fn InsertAt<P1>(&self, index: u32, value: P1) -> windows_core::Result<()>
                where
                    P1: windows_core::Param<T>,
                {
                    unsafe {
                        (windows_core::Interface::vtable(self).InsertAt)(
                            windows_core::Interface::as_raw(self),
                            index,
                            value.param().abi(),
                        )
                        .ok()
                    }
                }
                pub fn RemoveAt(&self, index: u32) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).RemoveAt)(
                            windows_core::Interface::as_raw(self),
                            index,
                        )
                        .ok()
                    }
                }
                pub fn Append<P0>(&self, value: P0) -> windows_core::Result<()>
                where
                    P0: windows_core::Param<T>,
                {
                    unsafe {
                        (windows_core::Interface::vtable(self).Append)(
                            windows_core::Interface::as_raw(self),
                            value.param().abi(),
                        )
                        .ok()
                    }
                }
                pub fn RemoveAtEnd(&self) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).RemoveAtEnd)(
                            windows_core::Interface::as_raw(self),
                        )
                        .ok()
                    }
                }
                pub fn Clear(&self) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).Clear)(
                            windows_core::Interface::as_raw(self),
                        )
                        .ok()
                    }
                }
                pub fn GetMany(
                    &self,
                    startindex: u32,
                    items: &mut [<T as windows_core::imp::Type<T>>::Default],
                ) -> windows_core::Result<u32> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).GetMany)(
                            windows_core::Interface::as_raw(self),
                            startindex,
                            items.len().try_into().unwrap(),
                            core::mem::transmute_copy(&items),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn ReplaceAll(
                    &self,
                    items: &[<T as windows_core::imp::Type<T>>::Default],
                ) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).ReplaceAll)(
                            windows_core::Interface::as_raw(self),
                            items.len().try_into().unwrap(),
                            core::mem::transmute(items.as_ptr()),
                        )
                        .ok()
                    }
                }
                pub fn First(&self) -> windows_core::Result<windows_collections::IIterator<T>> {
                    let this = &windows_core::Interface::cast::<IIterable<T>>(self)?;
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(this).First)(
                            windows_core::Interface::as_raw(this),
                            &mut result__,
                        )
                        .and_then(|| windows_core::imp::Type::from_abi(result__))
                    }
                }
            }
            impl<T: windows_core::RuntimeType + 'static> IntoIterator for IVector<T> {
                type Item = T;
                type IntoIter = windows_collections::BufferedIterator<Self::Item>;
                fn into_iter(self) -> Self::IntoIter {
                    IntoIterator::into_iter(&self)
                }
            }
            impl<T: windows_core::RuntimeType + 'static> IntoIterator for &IVector<T> {
                type Item = T;
                type IntoIter = windows_collections::BufferedIterator<Self::Item>;
                fn into_iter(self) -> Self::IntoIter {
                    windows_collections::BufferedIterator::new(self.First().unwrap())
                }
            }
            #[repr(C)]
            pub struct IVector_Vtbl<T>
            where
                T: windows_core::RuntimeType + 'static,
            {
                pub base__: windows_core::IInspectable_Vtbl,
                pub GetAt: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    u32,
                    *mut windows_core::imp::AbiType<T>,
                ) -> windows_core::HRESULT,
                pub Size: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut u32,
                ) -> windows_core::HRESULT,
                pub GetView: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut *mut core::ffi::c_void,
                ) -> windows_core::HRESULT,
                pub IndexOf: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    windows_core::imp::AbiType<T>,
                    *mut u32,
                    *mut bool,
                ) -> windows_core::HRESULT,
                pub SetAt: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    u32,
                    windows_core::imp::AbiType<T>,
                ) -> windows_core::HRESULT,
                pub InsertAt: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    u32,
                    windows_core::imp::AbiType<T>,
                ) -> windows_core::HRESULT,
                pub RemoveAt:
                    unsafe extern "system" fn(*mut core::ffi::c_void, u32) -> windows_core::HRESULT,
                pub Append: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    windows_core::imp::AbiType<T>,
                ) -> windows_core::HRESULT,
                pub RemoveAtEnd:
                    unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
                pub Clear:
                    unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
                pub GetMany: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    u32,
                    u32,
                    *mut windows_core::imp::AbiType<T>,
                    *mut u32,
                ) -> windows_core::HRESULT,
                pub ReplaceAll: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    u32,
                    *const windows_core::imp::AbiType<T>,
                ) -> windows_core::HRESULT,
                T: core::marker::PhantomData<T>,
            }
        }
    }
    pub mod Graphics {
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
        pub struct SizeInt32 {
            pub Width: i32,
            pub Height: i32,
        }
        impl windows_core::imp::TypeKind for SizeInt32 {
            type TypeKind = windows_core::imp::CopyType;
        }
        impl windows_core::RuntimeType for SizeInt32 {
            const SIGNATURE: windows_core::imp::ConstBuffer =
                windows_core::imp::ConstBuffer::from_slice(
                    b"struct(Windows.Graphics.SizeInt32;i4;i4)",
                );
            const NAME: windows_core::imp::ConstBuffer =
                windows_core::imp::ConstBuffer::from_slice(b"Windows.Graphics.SizeInt32");
        }
    }
    pub mod Storage {
        pub mod Streams {
            windows_core::imp::define_interface!(
                IBuffer,
                IBuffer_Vtbl,
                0x905a0fe0_bc53_11df_8c49_001e4fc686da
            );
            impl windows_core::RuntimeType for IBuffer {
                const SIGNATURE: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::for_interface::<Self>();
                const NAME: windows_core::imp::ConstBuffer =
                    windows_core::imp::ConstBuffer::from_slice(b"Windows.Storage.Streams.IBuffer");
            }
            windows_core::imp::interface_hierarchy!(
                IBuffer,
                windows_core::IUnknown,
                windows_core::IInspectable
            );
            impl IBuffer {
                pub fn Capacity(&self) -> windows_core::Result<u32> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Capacity)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn Length(&self) -> windows_core::Result<u32> {
                    unsafe {
                        let mut result__ = core::mem::zeroed();
                        (windows_core::Interface::vtable(self).Length)(
                            windows_core::Interface::as_raw(self),
                            &mut result__,
                        )
                        .map(|| result__)
                    }
                }
                pub fn SetLength(&self, value: u32) -> windows_core::Result<()> {
                    unsafe {
                        (windows_core::Interface::vtable(self).SetLength)(
                            windows_core::Interface::as_raw(self),
                            value,
                        )
                        .ok()
                    }
                }
            }
            #[repr(C)]
            pub struct IBuffer_Vtbl {
                pub base__: windows_core::IInspectable_Vtbl,
                pub Capacity: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut u32,
                ) -> windows_core::HRESULT,
                pub Length: unsafe extern "system" fn(
                    *mut core::ffi::c_void,
                    *mut u32,
                ) -> windows_core::HRESULT,
                pub SetLength:
                    unsafe extern "system" fn(*mut core::ffi::c_void, u32) -> windows_core::HRESULT,
            }
        }
    }
    pub mod UI {
        pub mod Xaml {
            pub mod Interop {
                #[repr(transparent)]
                #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
                pub struct TypeKind(pub i32);
                impl TypeKind {
                    pub const Primitive: Self = Self(0);
                    pub const Metadata: Self = Self(1);
                    pub const Custom: Self = Self(2);
                }
                impl windows_core::imp::TypeKind for TypeKind {
                    type TypeKind = windows_core::imp::CopyType;
                }
                impl windows_core::RuntimeType for TypeKind {
                    const SIGNATURE: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"enum(Windows.UI.Xaml.Interop.TypeKind;i4)",
                        );
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Windows.UI.Xaml.Interop.TypeKind",
                        );
                }
                #[repr(C)]
                #[derive(Clone, Debug, Default, Eq, PartialEq)]
                pub struct TypeName {
                    pub Name: windows_core::HSTRING,
                    pub Kind: TypeKind,
                }
                impl windows_core::imp::TypeKind for TypeName {
                    type TypeKind = windows_core::imp::CloneType;
                }
                impl windows_core::RuntimeType for TypeName {
                    const SIGNATURE : windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice (b"struct(Windows.UI.Xaml.Interop.TypeName;string;enum(Windows.UI.Xaml.Interop.TypeKind;i4))") ;
                    const NAME: windows_core::imp::ConstBuffer =
                        windows_core::imp::ConstBuffer::from_slice(
                            b"Windows.UI.Xaml.Interop.TypeName",
                        );
                }
            }
        }
    }
}
