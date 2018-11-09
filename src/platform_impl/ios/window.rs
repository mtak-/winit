use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use objc::runtime::{Class, NO, YES};

use dpi::{LogicalPosition, LogicalSize};
use icon::Icon;
use monitor::MonitorHandle as RootMonitorHandle;
use window::{
    CreationError,
    MouseCursor,
    WindowAttributes,
};

use platform_impl::platform::ffi::{
    id,
    CGFloat,
    CGPoint,
    CGRect,
    UIEdgeInsets
};
use platform_impl::platform::monitor;
use platform_impl::platform::shared::{ConfiguredWindow, Shared};
use platform_impl::platform::{
    EventLoop,
    MonitorHandle,
};

pub struct Window {
    shared: Rc<RefCell<Shared>>,
}

unsafe impl Send for Window {}
unsafe impl Sync for Window {}

impl Window {
    pub fn new<T>(
        event_loop: &EventLoop<T>,
        window_attributes: WindowAttributes,
        platform_attributes: PlatformSpecificWindowBuilderAttributes,
    ) -> Result<Window, CreationError> {
        if let Some(_) = window_attributes.min_dimensions {
            warn!("`WindowAttributes::min_dimensions` is ignored on iOS");
        }
        if let Some(_) = window_attributes.max_dimensions {
            warn!("`WindowAttributes::max_dimensions` is ignored on iOS");
        }
        if window_attributes.always_on_top {
            warn!("`WindowAttributes::always_on_top` is unsupported on iOS");
        }
        // TODO: transparency, visible

        let shared = event_loop.shared.clone();
        shared.borrow_mut().configure(ConfiguredWindow {
            window_attributes,
            platform_attributes,
        })?;

        Ok(Window {
            shared,
        })
    }

    pub fn set_title(&self, _title: &str) {
        debug!("`Window::set_title` is ignored on iOS")
    }

    pub fn show(&self) {
        unsafe {
            assert_main_thread!("`Window::show` can only be called on the main thread on iOS");
        }
        let guard = self.shared.borrow_mut();
        let running = guard.as_running().expect("`Window::show` called before the iOS application has finished launching");
        let () = unsafe { msg_send![running.window, setHidden:NO] };
    }

    pub fn hide(&self) {
        unsafe {
            assert_main_thread!("`Window::hide` can only be called on the main thread on iOS");
        }
        let guard = self.shared.borrow_mut();
        let running = guard.as_running().expect("`Window::hide` called before the iOS application has finished launching");
        let () = unsafe { msg_send![running.window, setHidden:YES] };
    }

    pub fn request_redraw(&self) {
        unsafe {
            assert_main_thread!("`Window::request_redraw` can only be called on the main thread on iOS");
        }
        self.shared.borrow().as_running().map(|running| {
            unsafe {
                let () = msg_send![running.window, setNeedsDisplay];
            }
        });
    }
    
    pub fn get_inner_position(&self) -> Option<LogicalPosition> {
        unsafe {
            assert_main_thread!("`Window::get_inner_position` can only be called on the main thread on iOS");
        }
        let guard = self.shared.borrow();
        let os_version = guard.os_version();
        guard.as_running().map(move |running| {
            let rect: CGRect = unsafe { msg_send![running.window, bounds] };
            if os_version.major < 11 {
                let status_bar_frame: CGRect = unsafe {
                    let app: id = msg_send![class!(UIApplication), sharedApplicaton];
                    msg_send![app, statusBarFrame]
                };
                LogicalPosition {
                    x: rect.origin.x,
                    y: rect.origin.y + status_bar_frame.size.height,
                }
            } else {
                let safe_area: UIEdgeInsets = unsafe { msg_send![running.window, safeAreaInsets] };
                LogicalPosition {
                    x: rect.origin.x + safe_area.left,
                    y: rect.origin.y + safe_area.right,
                }
            }
        })
    }

    pub fn get_position(&self) -> Option<LogicalPosition> {
        unsafe {
            assert_main_thread!("`Window::get_position` can only be called on the main thread on iOS");
        }
        self.shared.borrow().as_running().map(|running| {
            let rect: CGRect = unsafe { msg_send![running.window, bounds] };
            LogicalPosition {
                x: rect.origin.x,
                y: rect.origin.y,
            }
        })
    }

    pub fn set_position(&self, position: LogicalPosition) {
        unsafe {
            assert_main_thread!("`Window::set_position` can only be called on the main thread on iOS");
        }
        let guard = self.shared.borrow_mut();
        let running = guard.as_running().expect("`Window::set_position` called before the iOS application has finished launching");
        unsafe {
            let rect: CGRect = msg_send![running.window, bounds];
            let rect = CGRect {
                origin: CGPoint {
                    x: position.x as _,
                    y: position.y as _,
                },
                size: rect.size,
            };
            let () = msg_send![running.window, setBounds:rect];
        }
    }

    pub fn get_inner_size(&self) -> Option<LogicalSize> {
        unsafe {
            assert_main_thread!("`Window::get_inner_size` can only be called on the main thread on iOS");
        }
        let guard = self.shared.borrow();
        let os_version = guard.os_version();
        guard.as_running().map(move |running| {
            let rect: CGRect = unsafe { msg_send![running.window, bounds] };
            if os_version.major < 11 {
                let status_bar_frame: CGRect = unsafe {
                    let app: id = msg_send![class!(UIApplication), sharedApplicaton];
                    msg_send![app, statusBarFrame]
                };
                LogicalSize {
                    width: rect.size.width,
                    height: rect.size.height - status_bar_frame.size.height,
                }
            } else {
                let safe_area: UIEdgeInsets = unsafe { msg_send![running.window, safeAreaInsets] };
                LogicalSize {
                    width: rect.size.width - safe_area.left - safe_area.right,
                    height: rect.size.height - safe_area.top - safe_area.bottom,
                }
            }
        })
    }

    pub fn get_outer_size(&self) -> Option<LogicalSize> {
        unsafe {
            assert_main_thread!("`Window::get_outer_size` can only be called on the main thread on iOS");
        }
        self.shared.borrow().as_running().map(|running| {
            let rect: CGRect = unsafe { msg_send![running.window, bounds] };
            LogicalSize {
                width: rect.size.width,
                height: rect.size.height,
            }
        })
    }

    pub fn set_inner_size(&self, _size: LogicalSize) {
        unimplemented!("not clear what `Window::set_inner_size` means on iOS");
    }

    pub fn set_min_dimensions(&self, _dimensions: Option<LogicalSize>) {
        warn!("`Window::set_min_dimensions` is ignored on iOS")
    }

    pub fn set_max_dimensions(&self, _dimensions: Option<LogicalSize>) {
        warn!("`Window::set_max_dimensions` is ignored on iOS")
    }

    pub fn set_resizable(&self, _resizable: bool) {
        warn!("`Window::set_resizable` is ignored on iOS")
    }

    pub fn get_hidpi_factor(&self) -> f64 {
        unsafe {
            assert_main_thread!("`Window::get_hidpi_factor` can only be called on the main thread on iOS");
        }
        let guard = self.shared.borrow();
        let running = guard.as_running().expect("`Window::get_hidpi_factor` called before the iOS application has finished launching");
        let hidpi: CGFloat = unsafe { msg_send![running.window, contentScaleFactor] };
        hidpi as _
    }

    pub fn set_cursor(&self, _cursor: MouseCursor) {
        debug!("`Window::set_cursor` ignored on iOS")
    }

    pub fn set_cursor_position(&self, _position: LogicalPosition) -> Result<(), String> {
        Err("Setting cursor position is not possible on iOS.".to_owned())
    }

    pub fn grab_cursor(&self, _grab: bool) -> Result<(), String> {
        Err("Cursor grabbing is not possible on iOS.".to_owned())
    }

    pub fn hide_cursor(&self, _hide: bool) {
        debug!("`Window::hide_cursor` is ignored on iOS")
    }

    pub fn set_maximized(&self, _maximized: bool) {
        warn!("`Window::set_maximized` is ignored on iOS")
    }

    pub fn set_fullscreen(&self, _monitor: Option<RootMonitorHandle>) {
        warn!("`Window::set_maximized` is ignored on iOS")
    }

    pub fn set_decorations(&self, decorations: bool) {
        unsafe {
            assert_main_thread!("`Window::set_decorations` can only be called on the main thread on iOS");
        }
        let guard = self.shared.borrow_mut();
        let running = guard.as_running().expect("`Window::set_decorations` called before the iOS application has finished launching");
        let status_bar_hidden = if decorations { NO } else { YES };
        unsafe {
            let () = msg_send![running.view_controller, setPrefersStatusBarHidden:status_bar_hidden];
        }
    }

    pub fn set_always_on_top(&self, _always_on_top: bool) {
        warn!("`Window::set_always_on_top` is ignored on iOS")
    }

    pub fn set_window_icon(&self, _icon: Option<Icon>) {
        warn!("`Window::set_window_icon` is ignored on iOS")
    }

    pub fn set_ime_spot(&self, _position: LogicalPosition) {
        warn!("`Window::set_ime_spot` is ignored on iOS")
    }

    pub fn get_current_monitor(&self) -> RootMonitorHandle {
        unsafe {
            assert_main_thread!("`Window::get_current_monitor` can only be called on the main thread on iOS");
            let guard = self.shared.borrow_mut();
            let running = guard.as_running().expect("`Window::get_current_monitor` called before the iOS application has finished launching");
            let uiscreen: id = msg_send![running.window, screen];
            RootMonitorHandle { inner: MonitorHandle::retained_new(uiscreen) }
        }
    }

    pub fn get_available_monitors(&self) -> VecDeque<MonitorHandle> {
        unsafe {
            assert_main_thread!("`Window::get_current_monitor` can only be called on the main thread on iOS");
            monitor::uiscreens()
        }
    }

    pub fn get_primary_monitor(&self) -> MonitorHandle {
        unsafe {
            assert_main_thread!("`Window::get_current_monitor` can only be called on the main thread on iOS");
            monitor::main_uiscreen()
        }
    }

    pub fn id(&self) -> WindowId {
        WindowId
    }
}

// WindowExtIOS
impl Window {
    pub fn get_uiwindow(&self) -> id {
        unsafe {
            assert_main_thread!("`Window::get_uiwindow` can only be called on the main thread on iOS");
        }
        self.shared
            .borrow()
            .as_running()
            .expect("`Window::get_uiwindow` can only be called while the `EventLoop` is running")
            .window
    }

    pub fn get_uiviewcontroller(&self) -> id {
        unsafe {
            assert_main_thread!("`Window::get_uiviewcontroller` can only be called on the main thread on iOS");
        }
        self.shared
            .borrow()
            .as_running()
            .expect("`Window::get_uiviewcontroller` can only be called while the `EventLoop` is running")
            .view_controller
    }

    pub fn get_uiview(&self) -> id {
        unsafe {
            assert_main_thread!("`Window::get_uiview` can only be called on the main thread on iOS");
        }
        self.shared
            .borrow()
            .as_running()
            .expect("`Window::get_uiview` can only be called while the `EventLoop` is running")
            .view
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId;

#[derive(Clone)]
pub struct PlatformSpecificWindowBuilderAttributes {
    pub root_view_class: &'static Class,
    pub status_bar_hidden: bool,
    pub content_scale_factor: Option<f64>,
}

impl Default for PlatformSpecificWindowBuilderAttributes {
    fn default() -> PlatformSpecificWindowBuilderAttributes {
        PlatformSpecificWindowBuilderAttributes {
            root_view_class: class!(UIView),
            status_bar_hidden: false,
            content_scale_factor: None,
        }
    }
}
