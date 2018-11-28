use std::collections::VecDeque;
use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::{mem, ptr};
use std::sync::mpsc::{self, Sender, Receiver};
use std::time::Instant;

use event::{Event, StartCause};
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
    CFAbsoluteTimeGetCurrent,
    CFRelease,
    CFRunLoopActivity,
    CFRunLoopAddObserver,
    CFRunLoopAddSource,
    CFRunLoopAddTimer,
    CFRunLoopGetMain,
    CFRunLoopObserverCreate,
    CFRunLoopObserverRef,
    CFRunLoopRef,
    CFRunLoopSourceContext,
    CFRunLoopSourceCreate,
    CFRunLoopSourceInvalidate,
    CFRunLoopSourceRef,
    CFRunLoopSourceSignal,
    CFRunLoopTimerCreate,
    CFRunLoopTimerInvalidate,
    CFRunLoopTimerRef,
    CFRunLoopTimerSetNextFireDate,
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
    pub app_state: RefCell<AppState>,
    receiver: Receiver<T>,
    waker: EventLoopWaker,
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

        let app_state = RefCell::default();
        let (sender_to_clone, receiver) = mpsc::channel();

        // this line sets up the main run loop before `UIApplicationMain`
        setup_control_flow_observers();
        
        let waker = EventLoopWaker::new(unsafe { CFRunLoopGetMain() });

        EventLoop {
            window_target: RootEventLoopWindowTarget {
                p: EventLoopWindowTarget {
                    app_state,
                    receiver,
                    waker,
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
            debug_assert!(EVENT_HANDLER.is_none(), "multiple `EventLoop`s are unsupported on iOS");
            EVENT_HANDLER = Some(Box::into_raw(Box::new(EventLoopHandler::new(
                event_handler,
                self.window_target,
            ))));

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
                    let callback = &mut *EVENT_HANDLER.expect("attempt to process an event without a running `EventLoop`");
                    callback.handle_user_events();
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

static mut EVENT_HANDLER: Option<*mut EventHandler> = None;

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
                    kCFRunLoopAfterWaiting => process_erased_event(RawEvent::WaitCancelled),
                    kCFRunLoopEntry => process_erased_event(RawEvent::Poll),
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
                    kCFRunLoopBeforeWaiting => process_erased_event(RawEvent::EventsCleared),
                    kCFRunLoopExit => process_erased_event(RawEvent::Exit),
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

// requires main thread
pub unsafe fn did_finish_launching() {
    let mut app_state = (&*EVENT_HANDLER.unwrap()).app_state().borrow_mut();
    app_state.did_finish_launching()
}

#[derive(Debug)]
pub enum RawEvent {
    Init,
    WaitCancelled,
    EventsCleared,

    Poll,
    Exit,

    Event(Event<()>),
}

impl From<Event<()>> for RawEvent {
    fn from(event: Event<()>) -> RawEvent {
        RawEvent::Event(event)
    }
}

trait EventHandler {
    fn handle_nonuser_event(&mut self, event: RawEvent);
    fn handle_user_events(&mut self);
    fn app_state(&self) -> &RefCell<AppState>;
}

#[derive(Debug, PartialEq, Eq)]
enum LoopState {
    Uninitialized,
    NotRunning,
    Running {
        active_control_flow: ControlFlow,
    },
}

impl LoopState {
    fn is_running(&self) -> bool {
        match self {
            &LoopState::Running{..} => true,
            _ => false,
        }
    }
}

struct EventLoopHandler<F, T: 'static> {
    f: F,
    event_loop: RootEventLoopWindowTarget<T>,
    control_flow: ControlFlow,
    loop_state: LoopState,
    start: Option<Instant>,
}

macro_rules! bug {
    ($msg:expr) => {
        panic!("winit iOS bug, file an issue: {}", $msg)
    };
}

macro_rules! debug_bug {
    ($msg:expr) => {
        if cfg!(debug_assertions) {
            bug!($msg)
        }
    };
}

macro_rules! debug_bug_assert {
    ($e:expr, $msg:expr) => {
        debug_assert!($e, "winit iOS bug, file an issue: {}", $msg)
    };
}

macro_rules! debug_bug_assert_eq {
    ($e0:expr, $e1:expr, $msg:expr) => {
        debug_assert_eq!($e0, $e1, "winit iOS bug, file an issue: {}", $msg)
    };
}

impl<F, T> EventLoopHandler<F, T>
where
    F: 'static + FnMut(Event<T>, &RootEventLoopWindowTarget<T>, &mut ControlFlow),
    T: 'static,
{
    fn new(f: F, event_loop: RootEventLoopWindowTarget<T>) -> EventLoopHandler<F, T> {
        EventLoopHandler {
            f,
            event_loop,
            control_flow: ControlFlow::default(),
            loop_state: LoopState::Uninitialized,
            start: None,
        }
    }

    fn handle_loop_state_transition(&mut self, raw_event: RawEvent) -> (Event<T>, Option<ControlFlow>) {
        (match (raw_event, self.control_flow) {
            (r, ControlFlow::Exit) | (r@RawEvent::Exit, _) | (r@RawEvent::Poll, _) => {
                bug!(format!("unexpected ControlFlow {:?} {:?}", r, self.control_flow))
            }
            (RawEvent::Init, ControlFlow::Poll) => {
                debug_bug_assert_eq!(self.loop_state, LoopState::Uninitialized, "`Init` sent more than once");
                debug_bug_assert!(self.start.is_none(), "`EventHandler` has an unexpected `start` time");
                self.loop_state = LoopState::Running {
                    active_control_flow: ControlFlow::Poll
                };
                self.event_loop.p.waker.start();
                Event::NewEvents(StartCause::Init)
            }
            (RawEvent::Init, other) => bug!(format!("`Init` sent with unexpected control flow {:?}", other)),
            (RawEvent::WaitCancelled, ControlFlow::Poll) => {
                debug_bug_assert_eq!(self.loop_state, LoopState::NotRunning, "Resuming a resumed EventLoop");
                debug_bug_assert!(self.start.is_none(), "`EventHandler` has an unexpected `start` time");
                self.loop_state = LoopState::Running {
                    active_control_flow: ControlFlow::Poll
                };
                Event::NewEvents(StartCause::Poll)
            }
            (RawEvent::WaitCancelled, ControlFlow::Wait) => {
                debug_bug_assert_eq!(self.loop_state, LoopState::NotRunning, "Resuming a resumed EventLoop");
                self.loop_state = LoopState::Running {
                    active_control_flow: ControlFlow::Wait
                };
                Event::NewEvents(StartCause::WaitCancelled {
                    start: self.start.take().expect("winit iOS bug, file an issue: `EventHandler` expected `start` time, found `None`"),
                    requested_resume: None,
                })
            }
            (RawEvent::WaitCancelled, ControlFlow::WaitUntil(requested_resume)) => {
                debug_bug_assert_eq!(self.loop_state, LoopState::NotRunning, "Resuming a resumed EventLoop");
                self.loop_state = LoopState::Running {
                    active_control_flow: ControlFlow::WaitUntil(requested_resume)
                };
                let start = self.start.take().expect("winit iOS bug, file an issue: `EventHandler` expected `start` time, found `None`");
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
            (RawEvent::EventsCleared, _) => {
                debug_bug_assert!(self.start.is_none(), "`EventHandler` has an unexpected `start` time");
                if let LoopState::Running { active_control_flow } = self.loop_state {
                    self.loop_state = LoopState::NotRunning;
                    return (Event::EventsCleared, Some(active_control_flow))
                } else {
                    bug!("`EventsCleared` sent for an `EventLoop` that is not currently processing events")
                }
            }
            (RawEvent::Event(event), _) => {
                debug_bug_assert!(self.loop_state.is_running(), "`Event` sent for an `EventLoop` that is not currently processing events");
                debug_bug_assert!(self.start.is_none(), "`EventHandler` has an unexpected `start` time");
                event.map_nonuser_event().expect("Failed to `map_nonuser_event`")
            }
        }, None)
    }

    fn handle_control_flow_transition(&mut self, old: ControlFlow) {
        let new = self.control_flow;
        match (old, new) {
            (ControlFlow::Poll, ControlFlow::Poll) => {}
            (ControlFlow::Wait, ControlFlow::Wait) => self.start = Some(Instant::now()),
            (ControlFlow::WaitUntil(old_instant), ControlFlow::WaitUntil(new_instant))
                if old_instant == new_instant => self.start = Some(Instant::now()),
            (_, ControlFlow::Wait) => {
                self.start = Some(Instant::now());
                self.event_loop.p.waker.stop()
            }
            (_, ControlFlow::WaitUntil(new_instant)) => {
                self.start = Some(Instant::now());
                self.event_loop.p.waker.start_at(new_instant)
            }
            (_, ControlFlow::Poll) => self.event_loop.p.waker.start(),
            (_, ControlFlow::Exit) => {
                // https://developer.apple.com/library/archive/qa/qa1561/_index.html
                // it is not possible to quit an iOS app gracefully and programatically
                warn!("`ControlFlow::Exit` ignored on iOS");
                self.control_flow = old
            }
        }
    }
}

impl<F, T> EventHandler for EventLoopHandler<F, T>
where
    F: 'static + FnMut(Event<T>, &RootEventLoopWindowTarget<T>, &mut ControlFlow),
    T: 'static,
{
    fn handle_nonuser_event(&mut self, event: RawEvent) {
        match (&self.loop_state, event) {
            (LoopState::Uninitialized, RawEvent::Init) => {
                let (event, ecf) = self.handle_loop_state_transition(RawEvent::Init);
                debug_bug_assert!(ecf.is_none(), "Unexpected expiring `ControlFlow` during `Init`");
                (self.f)(
                    event,
                    &self.event_loop,
                    &mut self.control_flow,
                );
                // handle any user events that came in before Init
                self.handle_user_events()
            }
            (LoopState::Uninitialized, _) => {
                // we ignore events until Init is sent (buffering up user events)
            }
            (_, event) => {
                let (event, expiring_control_flow) = self.handle_loop_state_transition(event);
                (self.f)(
                    event,
                    &self.event_loop,
                    &mut self.control_flow,
                );
                expiring_control_flow.map(move |expiring_control_flow| {
                    self.handle_control_flow_transition(expiring_control_flow)
                });
            }
        }
    }

    fn handle_user_events(&mut self) {
        match &self.loop_state {
            &LoopState::Uninitialized => {} // ignored, see handle_nonuser_event
            &LoopState::NotRunning => debug_bug!("User event sent for an `EventLoop` that is not currently processing events"),
            &LoopState::Running {..} => {
                debug_bug_assert!(self.start.is_none(), "`EventHandler` has an unexpected `start` time");
                for event in self.event_loop.p.receiver.try_iter() {
                    (self.f)(
                        Event::UserEvent(event),
                        &self.event_loop,
                        &mut self.control_flow,
                    );
                }
            }
        }
    }

    fn app_state(&self) -> &RefCell<AppState> {
        &self.event_loop.p.app_state
    }
}

// requires being run on main thread
pub unsafe fn process_erased_event<E: Into<RawEvent>>(event: E) {
    let callback = &mut *EVENT_HANDLER.expect("attempt to process an event without a running `EventLoop`");
    let event = event.into();
    if let RawEvent::Event(Event::LoopDestroyed) = event {
        EVENT_HANDLER = None;
        callback.handle_nonuser_event(event);
        ptr::drop_in_place(callback)
    } else {
        callback.handle_nonuser_event(event);
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
                let fsecs = duration.subsec_nanos() as f64 / 1_000_000_000.0 + duration.as_secs() as f64;
                CFRunLoopTimerSetNextFireDate(self.timer, current + fsecs)
            }
        }
    }
}

// must be called on main thread
pub unsafe fn get_idiom() -> Idiom {
    let device: id = msg_send![class!(UIDevice), currentDevice];
    let raw_idiom: UIUserInterfaceIdiom = msg_send![device, userInterfaceIdiom];
    raw_idiom.into()
}