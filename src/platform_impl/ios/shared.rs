use window::{CreationError, WindowAttributes};

use platform_impl::platform::PlatformSpecificWindowBuilderAttributes;
use platform_impl::platform::ffi::{id, NSOperatingSystemVersion};

#[derive(Clone, Debug)]
pub struct OSVersion {
    pub major: u32,
    pub minor: u32,
}

enum SharedImpl {
    UnconfiguredWindow,
    ConfiguredWindow(ConfiguredWindow),
    Running(Running),
}

pub struct Shared {
    shared: SharedImpl,
    os_version: OSVersion,
}

impl Default for Shared {
    fn default() -> Shared {
        let version: NSOperatingSystemVersion = unsafe {
            let process_info: id = msg_send![class!(NSProcessInfo), processInfo];
            msg_send![process_info, operatingSystemVersion]
        };
        let os_version = OSVersion {
            major: version.major as u32,
            minor: version.minor as u32,
        };
        assert!(os_version.major >= 8, "`winit` current requires iOS version 8 or greater");
        
        let shared = SharedImpl::UnconfiguredWindow;
        Shared {
            shared,
            os_version,
        }
    }
}

impl Shared {
    pub fn configure(&mut self, config: ConfiguredWindow) -> Result<(), CreationError> {
        match &mut self.shared {
            &mut SharedImpl::UnconfiguredWindow => {
                self.shared = SharedImpl::ConfiguredWindow(config);
                Ok(())
            }
            &mut SharedImpl::ConfiguredWindow(..) | SharedImpl::Running(..) => {
                Err(CreationError::OsError("only one `Window` is currently supported on iOS".to_owned()))
            }
        }
    }

    pub fn run<F>(&mut self, f: F)
    where
        F: FnOnce(&ConfiguredWindow) -> Running
    {
        let running = match &mut self.shared {
            &mut SharedImpl::UnconfiguredWindow => panic!("iOS requires a configured `Window` to begin running"),
            &mut SharedImpl::ConfiguredWindow(ref mut config) => {
                SharedImpl::Running(f(config))
            }
            &mut SharedImpl::Running(..) => panic!("attempt to run `EventLoop` more than once on iOS")
        };
        self.shared = running;
    }

    pub fn as_running(&self) -> Option<&Running> {
        match &self.shared {
            &SharedImpl::Running(ref r) => Some(r),
            _ => None,
        }
    }

    pub fn os_version(&self) -> &OSVersion {
        &self.os_version
    }
}

pub struct ConfiguredWindow {
    pub window_attributes: WindowAttributes,
    pub platform_attributes: PlatformSpecificWindowBuilderAttributes,
}

pub struct Running {
    pub window: id,
    pub view_controller: id,
    pub view: id,
}

impl Drop for Running {
    fn drop(&mut self) {
        unsafe {
            let () = msg_send![self.view, release];
            let () = msg_send![self.view_controller, release];
            let () = msg_send![self.window, release];
        }
    }
}
