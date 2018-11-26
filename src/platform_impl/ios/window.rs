use std::collections::VecDeque;

use objc::runtime::{Class, NO, Object, YES};

use dpi::{LogicalPosition, LogicalSize};
use icon::Icon;
use monitor::MonitorHandle as RootMonitorHandle;
use platform::ios::{MonitorHandleExtIOS, SupportedOrientations};
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
    CGSize,
    UIEdgeInsets,
};
use platform_impl::platform::monitor;
use platform_impl::platform::view;
use platform_impl::platform::{
    EventLoop,
    MonitorHandle,
};

pub struct Window {
    pub window: id,
    pub view_controller: id,
    pub view: id,
    supports_safe_area: bool,
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let () = msg_send![self.view, release];
            let () = msg_send![self.view_controller, release];
            let () = msg_send![self.window, release];
        }
    }
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

        unsafe {
            let screen = window_attributes.fullscreen
                .as_ref()
                .map(|screen| screen.get_uiscreen() as _)
                .unwrap_or_else(|| monitor::main_uiscreen().get_uiscreen());
            let bounds: CGRect = msg_send![screen, bounds];

            let bounds = match window_attributes.dimensions {
                Some(dim) => CGRect {
                    origin: bounds.origin,
                    size: CGSize { width: dim.width, height: dim.height },
                },
                None => bounds,
            };

            let view = view::create_view(&window_attributes, &platform_attributes, bounds.clone());
            let view_controller = view::create_view_controller(&platform_attributes, view);
            let window = view::create_window(&window_attributes, &platform_attributes, bounds, view_controller);

            let mut guard = event_loop.app_state.borrow_mut();
            let supports_safe_area = guard.capabilities().supports_safe_area;

            let result = Window {
                window,
                view_controller,
                view,
                supports_safe_area,
            };
            guard.set_key_window(window);
            Ok(result)
        }
    }

    pub fn set_title(&self, _title: &str) {
        debug!("`Window::set_title` is ignored on iOS")
    }

    pub fn show(&self) {
        unsafe {
            assert_main_thread!("`Window::show` can only be called on the main thread on iOS");
            let () = msg_send![self.window, setHidden:NO];
        }
    }

    pub fn hide(&self) {
        unsafe {
            assert_main_thread!("`Window::hide` can only be called on the main thread on iOS");
            let () = msg_send![self.window, setHidden:YES];
        }
    }

    pub fn request_redraw(&self) {
        unsafe {
            assert_main_thread!("`Window::request_redraw` can only be called on the main thread on iOS");
            let () = msg_send![self.window, setNeedsDisplay];
        }
    }
    
    pub fn get_inner_position(&self) -> Option<LogicalPosition> {
        unsafe {
            assert_main_thread!("`Window::get_inner_position` can only be called on the main thread on iOS");

            let rect: CGRect = msg_send![self.window, bounds];
            Some(if self.supports_safe_area {
                let safe_area: UIEdgeInsets = msg_send![self.window, safeAreaInsets];
                LogicalPosition {
                    x: rect.origin.x + safe_area.left,
                    y: rect.origin.y + safe_area.top,
                }
            } else {
                let status_bar_frame: CGRect = {
                    let app: id = msg_send![class!(UIApplication), sharedApplicaton];
                    msg_send![app, statusBarFrame]
                };
                let x = rect.origin.x;
                let y = if rect.origin.y > status_bar_frame.size.height {
                    rect.origin.y
                } else {
                    status_bar_frame.size.height
                };
                LogicalPosition { x, y }
            })
        }
    }

    pub fn get_position(&self) -> Option<LogicalPosition> {
        unsafe {
            assert_main_thread!("`Window::get_position` can only be called on the main thread on iOS");

            let rect: CGRect = msg_send![self.window, bounds];
            Some(LogicalPosition {
                x: rect.origin.x,
                y: rect.origin.y,
            })
        }
    }

    pub fn set_position(&self, position: LogicalPosition) {
        unsafe {
            assert_main_thread!("`Window::set_position` can only be called on the main thread on iOS");

            let rect: CGRect = msg_send![self.window, bounds];
            let rect = CGRect {
                origin: CGPoint {
                    x: position.x as _,
                    y: position.y as _,
                },
                size: rect.size,
            };
            let () = msg_send![self.window, setBounds:rect];
        }
    }

    pub fn get_inner_size(&self) -> Option<LogicalSize> {
        unsafe {
            assert_main_thread!("`Window::get_inner_size` can only be called on the main thread on iOS");
            let rect: CGRect = msg_send![self.window, bounds];
            Some(if self.supports_safe_area {
                let safe_area: UIEdgeInsets = msg_send![self.window, safeAreaInsets];
                LogicalSize {
                    width: rect.size.width - safe_area.left - safe_area.right,
                    height: rect.size.height - safe_area.top - safe_area.bottom,
                }
            } else {
                let status_bar_frame: CGRect = {
                    let app: id = msg_send![class!(UIApplication), sharedApplicaton];
                    msg_send![app, statusBarFrame]
                };
                let width = rect.size.width;
                let height = if rect.origin.y > status_bar_frame.size.height {
                    rect.size.height
                } else {
                    rect.size.height + rect.origin.y - status_bar_frame.size.height
                };
                LogicalSize { width, height }
            })
        }
    }

    pub fn get_outer_size(&self) -> Option<LogicalSize> {
        unsafe {
            assert_main_thread!("`Window::get_outer_size` can only be called on the main thread on iOS");
            let rect: CGRect = msg_send![self.window, bounds];
            Some(LogicalSize {
                width: rect.size.width,
                height: rect.size.height,
            })
        }
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
            let hidpi: CGFloat = msg_send![self.window, contentScaleFactor];
            hidpi as _
        }
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
            let status_bar_hidden = if decorations { NO } else { YES };
            let () = msg_send![self.view_controller, setPrefersStatusBarHidden:status_bar_hidden];
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
            let uiscreen: id = msg_send![self.window, screen];
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
        self.window.into()
    }
}

// WindowExtIOS
impl Window {
    pub fn get_uiwindow(&self) -> id { self.window }
    pub fn get_uiviewcontroller(&self) -> id { self.view_controller }
    pub fn get_uiview(&self) -> id { self.view }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId {
    window: id,
}

unsafe impl Send for WindowId {}
unsafe impl Sync for WindowId {}

impl From<&Object> for WindowId {
    fn from(window: &Object) -> WindowId {
        WindowId { window: window as *const _ as _ }
    }
}

impl From<id> for WindowId {
    fn from(window: id) -> WindowId {
        WindowId { window }
    }
}

#[derive(Clone)]
pub struct PlatformSpecificWindowBuilderAttributes {
    pub root_view_class: &'static Class,
    pub status_bar_hidden: bool,
    pub content_scale_factor: Option<f64>,
    pub supported_orientations: SupportedOrientations,
}

impl Default for PlatformSpecificWindowBuilderAttributes {
    fn default() -> PlatformSpecificWindowBuilderAttributes {
        PlatformSpecificWindowBuilderAttributes {
            root_view_class: class!(UIView),
            status_bar_hidden: false,
            content_scale_factor: None,
            supported_orientations: SupportedOrientations::LandscapeAndPortrait,
        }
    }
}