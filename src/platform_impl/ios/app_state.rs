use platform_impl::platform::ffi::{id, NSOperatingSystemVersion, NSUInteger};

enum AppStateImpl {
    QueuedKeyWindows(Vec<id>),
    DidFinishLaunching,
}

impl Drop for AppStateImpl {
    fn drop(&mut self) {
        match self {
            &mut AppStateImpl::QueuedKeyWindows(ref mut windows) => unsafe {
                for &mut window in windows {
                    let () = msg_send![window, release];
                }
            }
            _ => {}
        }
    }
}

pub struct AppState {
    app_state: AppStateImpl,
    capabilities: Capabilities,
}

impl Default for AppState {
    fn default() -> AppState {
        let app_state = AppStateImpl::QueuedKeyWindows(Vec::default());

        let version: NSOperatingSystemVersion = unsafe {
            let process_info: id = msg_send![class!(NSProcessInfo), processInfo];
            msg_send![process_info, operatingSystemVersion]
        };
        let capabilities = version.into();

        AppState {
            app_state,
            capabilities,
        }
    }
}

impl AppState {
    // requires main thread and window is a UIWindow
    // retains window
    pub unsafe fn set_key_window(&mut self, window: id) {
        match &mut self.app_state {
            &mut AppStateImpl::QueuedKeyWindows(ref mut windows) => {
                windows.push(window);
                msg_send![window, retain];
            }
            &mut AppStateImpl::DidFinishLaunching => msg_send![window, makeKeyAndVisible],
        }
    }

    pub unsafe fn did_finish_launching(&mut self) {
        {
            let windows = match &mut self.app_state {
                &mut AppStateImpl::QueuedKeyWindows(ref mut windows) => windows,
                &mut AppStateImpl::DidFinishLaunching => panic!("attempt to run `EventLoop` more than once on iOS"),
            };

            for idx in 0..windows.len() {
                let window = windows[idx];
                let count: NSUInteger = msg_send![window, retainCount];
                // make sure the window is still referenced
                if count > 1 {
                    // Do a little screen dance here to account for windows being created before
                    // `UIApplicationMain` is called. This fixes visual issues such as being offcenter
                    // and sized incorrectly. Additionally, to fix orientation issues, we gotta reset
                    // the `rootViewController`.
                    //
                    // relevant iOS log:
                    // ```
                    // [ApplicationLifecycle] Windows were created before application initialzation completed.
                    // This may result in incorrect visual appearance.
                    // ```
                    let screen: id = msg_send![window, screen];
                    let () = msg_send![screen, retain];
                    let () = msg_send![window, setScreen:0 as id];
                    let () = msg_send![window, setScreen:screen];
                    let () = msg_send![screen, release];
                    let controller: id = msg_send![window, rootViewController];
                    let () = msg_send![window, setRootViewController:0 as *const ()];
                    let () = msg_send![window, setRootViewController:controller];
                    let () = msg_send![window, makeKeyAndVisible];
                }
            }
        }
        self.app_state = AppStateImpl::DidFinishLaunching;
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
}

pub struct Capabilities {
    pub supports_safe_area: bool,
}

impl From<NSOperatingSystemVersion> for Capabilities {
    fn from(os_version: NSOperatingSystemVersion) -> Capabilities {
        assert!(os_version.major >= 8, "`winit` current requires iOS version 8 or greater");

        let supports_safe_area = os_version.major >= 11;

        Capabilities { supports_safe_area }
    }
}