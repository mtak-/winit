use std::collections::VecDeque;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::{mem, ptr};
use std::sync::mpsc::{self, Sender, Receiver};

use event::Event;
use event_loop::{
    ControlFlow,
    EventLoopWindowTarget as RootEventLoopWindowTarget,
    EventLoopClosed,
};

use platform::ios::Idiom;

use platform_impl::platform::app_state::AppState;
use platform_impl::platform::ffi::{
    id,
    nil,
    CFRelease,
    CFRunLoopActivity,
    CFRunLoopAddObserver,
    CFRunLoopAddSource,
    CFRunLoopGetMain,
    CFRunLoopObserverCreate,
    CFRunLoopObserverRef,
    CFRunLoopSourceContext,
    CFRunLoopSourceCreate,
    CFRunLoopSourceInvalidate,
    CFRunLoopSourceRef,
    CFRunLoopSourceSignal,
    CFRunLoopWakeUp,
    kCFRunLoopCommonModes,
    kCFRunLoopDefaultMode,
    kCFRunLoopEntry,
    kCFRunLoopBeforeWaiting,
    kCFRunLoopAfterWaiting,
    kCFRunLoopExit,
    NSString,
    UIApplicationMain,
    UIUserInterfaceIdiom,
};
use platform_impl::platform::monitor;
use platform_impl::platform::MonitorHandle;
use platform_impl::platform::view;

pub struct EventLoopWindowTarget<T: 'static> {
    receiver: Receiver<T>,
    sender_to_clone: Sender<T>,
}

pub struct EventLoop<T: 'static> {
    window_target: RootEventLoopWindowTarget<T>,
}

impl<T: 'static> EventLoop<T> {
    pub fn new() -> EventLoop<T> {
        static mut SINGLETON_INIT: bool = false;
        unsafe {
            assert_main_thread!("`EventLoop` can only be created on the main thread on iOS");
            assert!(!SINGLETON_INIT, "Only one `EventLoop` is supported on iOS. \
                `EventLoopProxy` might be helpful");
            SINGLETON_INIT = true;
            view::create_delegate_class();
        }

        let (sender_to_clone, receiver) = mpsc::channel();

        // this line sets up the main run loop before `UIApplicationMain`
        setup_control_flow_observers();

        EventLoop {
            window_target: RootEventLoopWindowTarget {
                p: EventLoopWindowTarget {
                    receiver,
                    sender_to_clone,
                },
                _marker: PhantomData,
            }
        }
    }

    pub fn run<F>(self, event_handler: F) -> !
    where
        F: 'static + FnMut(Event<T>, &RootEventLoopWindowTarget<T>, &mut ControlFlow)
    {
        unsafe {
            let application: *mut c_void = msg_send![class!(UIApplication), sharedApplication];
            assert_eq!(application, ptr::null_mut(), "\
                `EventLoop` cannot be `run` after a call to `UIApplicationMain` on iOS\n\
                Note: `EventLoop::run` calls `UIApplicationMain` on iOS");
            AppState::get_mut().will_launch(Box::new(EventLoopHandler {
                f: event_handler,
                event_loop: self.window_target,
            }));

            UIApplicationMain(0, ptr::null(), nil, NSString::alloc(nil).init_str("AppDelegate"));
            unreachable!()
        }
    }

    pub fn create_proxy(&self) -> EventLoopProxy<T> {
        EventLoopProxy::new(self.window_target.p.sender_to_clone.clone())
    }

    pub fn get_available_monitors(&self) -> VecDeque<MonitorHandle> {
        // guaranteed to be on main thread
        unsafe {
            monitor::uiscreens()
        }
    }

    pub fn get_primary_monitor(&self) -> MonitorHandle {
        // guaranteed to be on main thread
        unsafe {
            monitor::main_uiscreen()
        }
    }

    pub fn window_target(&self) -> &RootEventLoopWindowTarget<T> {
        &self.window_target
    }
}

// EventLoopExtIOS
impl<T: 'static> EventLoop<T> {
    pub fn get_idiom(&self) -> Idiom {
        // guaranteed to be on main thread
        unsafe {
            self::get_idiom()
        }
    }
}

pub struct EventLoopProxy<T> {
    sender: Sender<T>,
    source: CFRunLoopSourceRef,
}

unsafe impl<T> Send for EventLoopProxy<T> {}
unsafe impl<T> Sync for EventLoopProxy<T> {}

impl<T> Clone for EventLoopProxy<T> {
    fn clone(&self) -> EventLoopProxy<T> {
        EventLoopProxy::new(self.sender.clone())
    }
}

impl<T> Drop for EventLoopProxy<T> {
    fn drop(&mut self) {
        unsafe {
            CFRunLoopSourceInvalidate(self.source);
            CFRelease(self.source as _);
        }
    }
}

impl<T> EventLoopProxy<T> {
    fn new(sender: Sender<T>) -> EventLoopProxy<T> {
        unsafe {
            extern "C" fn event_loop_proxy_handler(_: *mut c_void) {
                unsafe {
                    AppState::get_mut().handle_user_events();
                }
            }

            // adding a Source to the main CFRunLoop lets us wake it up and
            // process user events through the normal OS EventLoop mechanisms.
            let rl = CFRunLoopGetMain();
            let mut context: CFRunLoopSourceContext = mem::zeroed();
            context.perform = event_loop_proxy_handler;
            let source = CFRunLoopSourceCreate(
                ptr::null_mut(),
                if cfg!(target_pointer_width = "32") {
                    (std::i32::MAX - 1) as _
                } else {
                    (std::i64::MAX - 1) as _
                },
                &mut context,
            );
            CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
            CFRunLoopWakeUp(rl);

            EventLoopProxy {
                sender,
                source,
            }
        }
    }

    pub fn send_event(&self, event: T) -> Result<(), EventLoopClosed> {
        self.sender.send(event).map_err(|_| EventLoopClosed)?;
        unsafe {
            // let the main thread know there's a new event
            CFRunLoopSourceSignal(self.source);
            let rl = CFRunLoopGetMain();
            CFRunLoopWakeUp(rl);
        }
        Ok(())
    }
}

fn setup_control_flow_observers() {
    unsafe {
        // begin is queued with the highest priority to ensure it is processed before other observers
        extern fn control_flow_begin_handler(
            _: CFRunLoopObserverRef,
            activity: CFRunLoopActivity,
            _: *mut c_void,
        ) {
            unsafe {
                #[allow(non_upper_case_globals)]
                match activity {
                    kCFRunLoopAfterWaiting => AppState::get_mut().handle_wakeup_transition(),
                    kCFRunLoopEntry => unimplemented!(), // not expected to ever happen
                    _ => unreachable!(),
                }
            }
        }

        // end is queued with the lowest priority to ensure it is processed after other observers
        // without that, LoopDestroyed will get sent after EventsCleared
        extern fn control_flow_end_handler(
            _: CFRunLoopObserverRef,
            activity: CFRunLoopActivity,
            _: *mut c_void,
        ) {
            unsafe {
                #[allow(non_upper_case_globals)]
                match activity {
                    kCFRunLoopBeforeWaiting => AppState::get_mut().handle_events_cleared(),
                    kCFRunLoopExit => unimplemented!(), // not expected to ever happen
                    _ => unreachable!(),
                }
            }
        }

        let main_loop = CFRunLoopGetMain();
        let begin_observer = CFRunLoopObserverCreate(
            ptr::null_mut(),
            kCFRunLoopEntry | kCFRunLoopAfterWaiting,
            1, // repeat = true
            if cfg!(target_pointer_width = "32") {
                std::i32::MIN as _
            } else {
                std::i64::MIN as _
            },
            control_flow_begin_handler,
            ptr::null_mut(),
        );
        CFRunLoopAddObserver(main_loop, begin_observer, kCFRunLoopDefaultMode);
        let end_observer = CFRunLoopObserverCreate(
            ptr::null_mut(),
            kCFRunLoopExit | kCFRunLoopBeforeWaiting,
            1, // repeat = true
            if cfg!(target_pointer_width = "32") {
                std::i32::MAX as _
            } else {
                std::i64::MAX as _
            },
            control_flow_end_handler,
            ptr::null_mut(),
        );
        CFRunLoopAddObserver(main_loop, end_observer, kCFRunLoopDefaultMode);
    }
}

pub trait EventHandler {
    fn handle_nonuser_event(&mut self, event: Event<()>, control_flow: &mut ControlFlow);
    fn handle_user_events(&mut self, control_flow: &mut ControlFlow);
}

struct EventLoopHandler<F, T: 'static> {
    f: F,
    event_loop: RootEventLoopWindowTarget<T>,
}

impl<F, T> EventHandler for EventLoopHandler<F, T>
where
    F: 'static + FnMut(Event<T>, &RootEventLoopWindowTarget<T>, &mut ControlFlow),
    T: 'static,
{
    fn handle_nonuser_event(&mut self, event: Event<()>, control_flow: &mut ControlFlow) {
        (self.f)(
            event.map_nonuser_event().expect("unexpectedly attempted to process a user event"),
            &self.event_loop,
            control_flow,
        );
    }

    fn handle_user_events(&mut self, control_flow: &mut ControlFlow) {
        for event in self.event_loop.p.receiver.try_iter() {
            (self.f)(
                Event::UserEvent(event),
                &self.event_loop,
                control_flow,
            );
        }
    }
}

// must be called on main thread
pub unsafe fn get_idiom() -> Idiom {
    let device: id = msg_send![class!(UIDevice), currentDevice];
    let raw_idiom: UIUserInterfaceIdiom = msg_send![device, userInterfaceIdiom];
    raw_idiom.into()
}