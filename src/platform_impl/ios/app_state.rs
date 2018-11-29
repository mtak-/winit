use std::cell::{RefCell, RefMut};
use std::mem::ManuallyDrop;
use std::os::raw::c_void;
use std::{mem, ptr};
use std::time::Instant;

use event::{Event, StartCause};
use event_loop::ControlFlow;
use platform_impl::platform::event_loop::EventHandler;
use platform_impl::platform::ffi::{
    id,
    CFAbsoluteTimeGetCurrent,
    CFRelease,
    CFRunLoopAddTimer,
    CFRunLoopGetMain,
    CFRunLoopRef,
    CFRunLoopTimerCreate,
    CFRunLoopTimerInvalidate,
    CFRunLoopTimerRef,
    CFRunLoopTimerSetNextFireDate,
    kCFRunLoopCommonModes,
    NSOperatingSystemVersion,
    NSUInteger,
};

macro_rules! bug {
    ($msg:expr) => {
        panic!("winit iOS bug, file an issue: {}", $msg)
    };
}

// this is the state machine for the app lifecycle
enum AppStateImpl {
    NotLaunched {
        queued_windows: Vec<id>,
        queued_events: Vec<Event<()>>,
    },
    Launching {
        queued_windows: Vec<id>,
        queued_events: Vec<Event<()>>,
        queued_event_handler: Box<EventHandler>,
    },
    ProcessingEvents {
        event_handler: Box<EventHandler>,
        active_control_flow: ControlFlow,
    },
    Waiting {
        waiting_event_handler: Box<EventHandler>,
        start: Instant,
    },
    PollFinished {
        waiting_event_handler: Box<EventHandler>,
    },
    Terminated,
}

impl Drop for AppStateImpl {
    fn drop(&mut self) {
        match self {
            &mut AppStateImpl::NotLaunched { ref mut queued_windows, .. } |
            &mut AppStateImpl::Launching { ref mut queued_windows, .. } => unsafe {
                for &mut window in queued_windows {
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
    control_flow: ControlFlow,
    waker: EventLoopWaker,
}

impl AppState {
    // requires main thread
    pub unsafe fn get_mut() -> RefMut<'static, AppState> {
        // basically everything in UIKit requires the main thread, so it's pointless to use the
        // std::sync APIs.
        static mut APP_STATE: RefCell<Option<AppState>> = RefCell::new(None);

        let mut guard = APP_STATE.borrow_mut();
        if guard.is_none() {
            #[inline(never)]
            #[cold]
            unsafe fn init_guard(guard: &mut RefMut<'static, Option<AppState>>) {
                let waker = EventLoopWaker::new(CFRunLoopGetMain());
                let version: NSOperatingSystemVersion = {
                    let process_info: id = msg_send![class!(NSProcessInfo), processInfo];
                    msg_send![process_info, operatingSystemVersion]
                };
                let capabilities = version.into();
                **guard = Some(AppState {
                    app_state: AppStateImpl::NotLaunched {
                        queued_windows: Vec::new(),
                        queued_events: Vec::new(),
                    },
                    capabilities,
                    control_flow: ControlFlow::default(),
                    waker,
                });
            }
            init_guard(&mut guard)
        }
        RefMut::map(guard, |state| {
            state.as_mut().unwrap()
        })
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
    
    // requires main thread and window is a UIWindow
    // retains window
    pub unsafe fn set_key_window(&mut self, window: id) {
        match &mut self.app_state {
            &mut AppStateImpl::NotLaunched { ref mut queued_windows, .. } => {
                queued_windows.push(window);
                msg_send![window, retain];
            }
            &mut AppStateImpl::ProcessingEvents { .. } => msg_send![window, makeKeyAndVisible],
            &mut AppStateImpl::Terminated => panic!("Attempt to create a `Window` \
                                                     after the app has terminated"),
            _ => unreachable!(), // all other cases should be impossible
        }
    }

    pub fn will_launch(&mut self, queued_event_handler: Box<EventHandler>) {
        unsafe {
            let (queued_windows, queued_events) = match &mut self.app_state {
                &mut AppStateImpl::NotLaunched {
                    ref mut queued_windows,
                    ref mut queued_events,
                } => {
                    let windows = ptr::read(queued_windows);
                    let events = ptr::read(queued_events);
                    (windows, events)
                }
                _ => panic!("winit iOS expected the app to be in a `NotLaunched` \
                             state, but was not - please file an issue"),
            };
            ptr::write(&mut self.app_state, AppStateImpl::Launching {
                queued_windows,
                queued_events,
                queued_event_handler,
            });
        }
    }

    pub fn did_finish_launching<'b>(mut this: RefMut<'b, Self>) {
        let (windows, events) = unsafe {
            let (windows, events, event_handler) = match &mut this.app_state {
                &mut AppStateImpl::Launching {
                    ref mut queued_windows,
                    ref mut queued_events,
                    ref mut queued_event_handler,
                } => {
                    let windows = ptr::read(queued_windows);
                    let events = ptr::read(queued_events);
                    let event_handler = ptr::read(queued_event_handler);
                    (windows, events, event_handler)
                }
                _ => panic!("winit iOS expected the app to be in a `Launching` \
                             state, but was not - please file an issue"),
            };
            ptr::write(&mut this.app_state, AppStateImpl::ProcessingEvents {
                event_handler,
                active_control_flow: ControlFlow::Poll,
            });
            (windows, events)
        };

        {
            let &mut AppState {
                ref mut app_state,
                ref mut control_flow,
                ..
            } = &mut *this;
            let event_handler = match app_state {
                &mut AppStateImpl::ProcessingEvents { ref mut event_handler, .. } => event_handler,
                _ => unreachable!(),
            };
            event_handler.handle_nonuser_event(Event::NewEvents(StartCause::Init), control_flow);
            for event in events {
                event_handler.handle_nonuser_event(event, control_flow)
            }
            event_handler.handle_user_events(control_flow);
        }

        drop(this);

        for window in windows {
            unsafe {
                let count: NSUInteger = msg_send![window, retainCount];
                // make sure the window is still referenced
                if count > 1 {
                    // Do a little screen dance here to account for windows being created before
                    // `UIApplicationMain` is called. This fixes visual issues such as being
                    // offcenter and sized incorrectly. Additionally, to fix orientation issues, we
                    // gotta reset the `rootViewController`.
                    //
                    // relevant iOS log:
                    // ```
                    // [ApplicationLifecycle] Windows were created before application initialzation
                    // completed. This may result in incorrect visual appearance.
                    // ```
                    let screen: id = msg_send![window, screen];
                    let () = msg_send![screen, retain];
                    let () = msg_send![window, setScreen:0 as id];
                    let () = msg_send![window, setScreen:screen];
                    let () = msg_send![screen, release];
                    let controller: id = msg_send![window, rootViewController];
                    let () = msg_send![window, setRootViewController:ptr::null::<()>()];
                    let () = msg_send![window, setRootViewController:controller];
                    let () = msg_send![window, makeKeyAndVisible];
                }
                let () = msg_send![window, release];
            }
        }
    }

    // AppState::did_finish_launching handles the special transition `Init`
    pub fn handle_wakeup_transition(&mut self) {
        let event = match self.control_flow {
            ControlFlow::Poll => {
                unsafe {
                    debug_assert_eq!(self.control_flow, ControlFlow::Poll);
                    let event_handler = match &mut self.app_state {
                        &mut AppStateImpl::NotLaunched { .. } |
                        &mut AppStateImpl::Launching { .. } => return,
                        &mut AppStateImpl::PollFinished {
                            ref mut waiting_event_handler,
                        } => ptr::read(waiting_event_handler),
                        _ => bug!("`EventHandler` unexpectedly started polling"),
                    };
                    ptr::write(&mut self.app_state, AppStateImpl::ProcessingEvents {
                        event_handler,
                        active_control_flow: ControlFlow::Poll,
                    });
                }
                Event::NewEvents(StartCause::Poll)
            }
            ControlFlow::Wait => {
                let start = unsafe {
                    let (event_handler, start) = match &mut self.app_state {
                        &mut AppStateImpl::NotLaunched { .. } |
                        &mut AppStateImpl::Launching { .. } => return,
                        &mut AppStateImpl::Waiting {
                            ref mut waiting_event_handler,
                            ref mut start,
                        } => (ptr::read(waiting_event_handler), *start),
                        _ => bug!("`EventHandler` unexpectedly woke up"),
                    };
                    ptr::write(&mut self.app_state, AppStateImpl::ProcessingEvents {
                        event_handler,
                        active_control_flow: ControlFlow::Wait,
                    });
                    start
                };
                Event::NewEvents(StartCause::WaitCancelled {
                    start,
                    requested_resume: None,
                })
            }
            ControlFlow::WaitUntil(requested_resume) => {
                let start = unsafe {
                    let (event_handler, start) = match &mut self.app_state {
                        &mut AppStateImpl::NotLaunched { .. } |
                        &mut AppStateImpl::Launching { .. } => return,
                        &mut AppStateImpl::Waiting {
                            ref mut waiting_event_handler,
                            ref mut start,
                        } => (ptr::read(waiting_event_handler), *start),
                        _ => bug!("`EventHandler` unexpectedly woke up"),
                    };
                    ptr::write(&mut self.app_state, AppStateImpl::ProcessingEvents {
                        event_handler,
                        active_control_flow: ControlFlow::WaitUntil(requested_resume),
                    });
                    start
                };
                if Instant::now() >= requested_resume {
                    Event::NewEvents(StartCause::ResumeTimeReached {
                        start,
                        requested_resume,
                    })
                } else {
                    Event::NewEvents(StartCause::WaitCancelled {
                        start,
                        requested_resume: Some(requested_resume),
                    })
                }
            }
            ControlFlow::Exit => bug!("unexpected controlflow `Exit`"),
        };
        match self {
            &mut AppState {
                app_state: AppStateImpl::ProcessingEvents { ref mut event_handler, .. },
                ref mut control_flow,
                ..
            } => event_handler.handle_nonuser_event(event, control_flow),
            _ => unreachable!(),
        }
    }

    pub fn handle_nonuser_event(&mut self, event: Event<()>) {
        match &mut self.app_state {
            &mut AppStateImpl::Launching {
                ref mut queued_events,
                ..
            }
            | &mut AppStateImpl::NotLaunched {
                ref mut queued_events,
                ..
            } => queued_events.push(event),
            &mut AppStateImpl::ProcessingEvents {
                ref mut event_handler,
                ..
            } => event_handler.handle_nonuser_event(event, &mut self.control_flow),
            &mut AppStateImpl::PollFinished { .. }
            | &mut AppStateImpl::Waiting { .. }
            | &mut AppStateImpl::Terminated => bug!("unexpected attempted to process an event"),
        }
    }

    pub fn handle_user_events(&mut self) {
        match &mut self.app_state {
            &mut AppStateImpl::Launching { .. } | &mut AppStateImpl::NotLaunched { .. } => return,
            &mut AppStateImpl::ProcessingEvents {
                ref mut event_handler,
                ..
            } => event_handler.handle_user_events(&mut self.control_flow),
            &mut AppStateImpl::PollFinished { .. }
            | &mut AppStateImpl::Waiting { .. }
            | &mut AppStateImpl::Terminated => bug!("unexpected attempted to process an event"),
        }
    }

    pub fn handle_events_cleared(&mut self) {
        let (event_handler, old) = match &mut self.app_state {
            &mut AppStateImpl::NotLaunched { .. } | &mut AppStateImpl::Launching { .. } => return,
            &mut AppStateImpl::ProcessingEvents {
                ref mut event_handler,
                ref mut active_control_flow,
            } => unsafe {
                (
                    ManuallyDrop::new(ptr::read(event_handler)),
                    *active_control_flow,
                )
            },
            _ => bug!("`EventHandler` expected to be processing events, but was not"),
        };

        let new = self.control_flow;
        match (old, new) {
            (ControlFlow::Poll, ControlFlow::Poll) => unsafe {
                ptr::write(
                    &mut self.app_state,
                    AppStateImpl::PollFinished {
                        waiting_event_handler: ManuallyDrop::into_inner(event_handler),
                    },
                )
            },
            (ControlFlow::Wait, ControlFlow::Wait) => unsafe {
                let start = Instant::now();
                ptr::write(
                    &mut self.app_state,
                    AppStateImpl::Waiting {
                        waiting_event_handler: ManuallyDrop::into_inner(event_handler),
                        start,
                    },
                )
            },
            (ControlFlow::WaitUntil(old_instant), ControlFlow::WaitUntil(new_instant))
                if old_instant == new_instant =>
            unsafe {
                let start = Instant::now();
                ptr::write(
                    &mut self.app_state,
                    AppStateImpl::Waiting {
                        waiting_event_handler: ManuallyDrop::into_inner(event_handler),
                        start,
                    },
                )
            }
            (_, ControlFlow::Wait) => unsafe {
                let start = Instant::now();
                ptr::write(
                    &mut self.app_state,
                    AppStateImpl::Waiting {
                        waiting_event_handler: ManuallyDrop::into_inner(event_handler),
                        start,
                    },
                );
                self.waker.stop()
            },
            (_, ControlFlow::WaitUntil(new_instant)) => unsafe {
                let start = Instant::now();
                ptr::write(
                    &mut self.app_state,
                    AppStateImpl::Waiting {
                        waiting_event_handler: ManuallyDrop::into_inner(event_handler),
                        start,
                    },
                );
                self.waker.start_at(new_instant)
            },
            (_, ControlFlow::Poll) => unsafe {
                ptr::write(
                    &mut self.app_state,
                    AppStateImpl::PollFinished {
                        waiting_event_handler: ManuallyDrop::into_inner(event_handler),
                    },
                );
                self.waker.start()
            },
            (_, ControlFlow::Exit) => {
                // https://developer.apple.com/library/archive/qa/qa1561/_index.html
                // it is not possible to quit an iOS app gracefully and programatically
                warn!("`ControlFlow::Exit` ignored on iOS");
                self.control_flow = old
            }
        }
        match self {
            &mut AppState {
                app_state:
                    AppStateImpl::PollFinished {
                        ref mut waiting_event_handler,
                        ..
                    },
                ref mut control_flow,
                ..
            }
            | &mut AppState {
                app_state:
                    AppStateImpl::Waiting {
                        ref mut waiting_event_handler,
                        ..
                    },
                ref mut control_flow,
                ..
            } => waiting_event_handler.handle_nonuser_event(Event::EventsCleared, control_flow),
            _ => unreachable!(),
        }
    }

    pub fn terminated<'a>(mut this: RefMut<'a, AppState>) {
        let mut old = mem::replace(&mut this.app_state, AppStateImpl::Terminated);
        if let AppStateImpl::ProcessingEvents { ref mut event_handler, .. } = old {
            event_handler.handle_nonuser_event(Event::LoopDestroyed, &mut this.control_flow)
        } else {
            bug!("`LoopDestroyed` happened while not processing events")
        }
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

struct EventLoopWaker {
    timer: CFRunLoopTimerRef,
}

impl Drop for EventLoopWaker {
    fn drop(&mut self) {
        unsafe {
            CFRunLoopTimerInvalidate(self.timer);
            CFRelease(self.timer as _);
        }
    }
}

impl EventLoopWaker {
    fn new(rl: CFRunLoopRef) -> EventLoopWaker {
        extern fn wakeup_main_loop(_timer: CFRunLoopTimerRef, _info: *mut c_void) {}
        unsafe {
            // create a timer with a 1microsec interval (1ns does not work) to mimic polling.
            // it is initially setup with a first fire time really far into the
            // future, but that gets changed to fire immediatley in did_finish_launching
            let timer = CFRunLoopTimerCreate(
                ptr::null_mut(),
                std::f64::MAX,
                0.000_000_1,
                0,
                0,
                wakeup_main_loop,
                ptr::null_mut(),
            );
            CFRunLoopAddTimer(rl, timer, kCFRunLoopCommonModes);

            EventLoopWaker { timer }
        }
    }

    fn stop(&mut self) {
        unsafe { CFRunLoopTimerSetNextFireDate(self.timer, std::f64::MAX) }
    }

    fn start(&mut self) {
        unsafe { CFRunLoopTimerSetNextFireDate(self.timer, std::f64::MIN) }
    }

    fn start_at(&mut self, instant: Instant) {
        let now = Instant::now();
        if now >= instant {
            self.start();
        } else {
            unsafe {
                let current = CFAbsoluteTimeGetCurrent();
                let duration = instant - now;
                let fsecs =
                    duration.subsec_nanos() as f64 / 1_000_000_000.0 + duration.as_secs() as f64;
                CFRunLoopTimerSetNextFireDate(self.timer, current + fsecs)
            }
        }
    }
}
