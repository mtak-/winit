#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use std::ffi::CString;
use std::os::raw::*;

use objc::{Encode, Encoding};
use objc::runtime::Object;

pub type id = *mut Object;
pub const nil: id = 0 as id;

#[cfg(target_pointer_width = "32")]
pub type CGFloat = f32;
#[cfg(target_pointer_width = "64")]
pub type CGFloat = f64;

#[cfg(target_pointer_width = "32")]
pub type NSInteger = i32;
#[cfg(target_pointer_width = "64")]
pub type NSInteger = i64;

#[cfg(target_pointer_width = "32")]
pub type NSUInteger = u32;
#[cfg(target_pointer_width = "64")]
pub type NSUInteger = u64;

#[repr(C)]
#[derive(Clone, Debug)]
pub struct NSOperatingSystemVersion {
    pub major: NSInteger,
    pub minor: NSInteger,
    pub patch: NSInteger,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct CGPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct CGSize {
    pub width: CGFloat,
    pub height: CGFloat,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

#[cfg(target_pointer_width = "32")]
unsafe impl Encode for CGRect {
    fn encode() -> Encoding {
        unsafe {
            Encoding::from_str("{CGRect={CGPoint=ff}{CGSize=ff}}")
        }
    }
}

#[cfg(target_pointer_width = "64")]
unsafe impl Encode for CGRect {
    fn encode() -> Encoding {
        unsafe {
            Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}")
        }
    }
}

#[cfg(target_pointer_width = "32")]
#[derive(Debug)]
#[allow(dead_code)]
#[repr(i32)]
pub enum UITouchPhase {
    Began = 0,
    Moved,
    Stationary,
    Ended,
    Cancelled,
}

#[cfg(target_pointer_width = "64")]
#[derive(Debug)]
#[allow(dead_code)]
#[repr(i64)]
pub enum UITouchPhase {
    Began = 0,
    Moved,
    Stationary,
    Ended,
    Cancelled,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct UIEdgeInsets {
    pub top: CGFloat,
    pub left: CGFloat,
    pub bottom: CGFloat,
    pub right: CGFloat,
}

#[link(name = "UIKit", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
extern {
    pub static kCFRunLoopDefaultMode: CFRunLoopMode;
    pub static kCFRunLoopCommonModes: CFRunLoopMode;

    pub fn UIApplicationMain(
        argc: c_int,
        argv: *const c_char,
        principalClassName: id,
        delegateClassName: id,
    ) -> c_int;

    pub fn CFRunLoopGetMain() -> CFRunLoopRef;
    pub fn CFRunLoopWakeUp(rl: CFRunLoopRef);

    pub fn CFRunLoopObserverCreate(
        allocator: CFAllocatorRef,
        activities: CFOptionFlags,
        repeats: Boolean,
        order: CFIndex,
        callout: CFRunLoopObserverCallBack,
        context: *mut CFRunLoopObserverContext,
    ) -> CFRunLoopObserverRef;
    pub fn CFRunLoopAddObserver(
        rl: CFRunLoopRef,
        observer: CFRunLoopObserverRef,
        mode: CFRunLoopMode,
    );

    pub fn CFRunLoopTimerCreate(
        allocator: CFAllocatorRef,
        fireDate: CFAbsoluteTime,
        interval: CFTimeInterval,
        flags: CFOptionFlags,
        order: CFIndex,
        callout: CFRunLoopTimerCallBack,
        context: *mut CFRunLoopTimerContext,
    ) -> CFRunLoopTimerRef;
    pub fn CFRunLoopAddTimer(
        rl: CFRunLoopRef,
        timer: CFRunLoopTimerRef,
        mode: CFRunLoopMode,
    );
    pub fn CFRunLoopTimerSetNextFireDate(
        timer: CFRunLoopTimerRef,
        fireDate: CFAbsoluteTime,
    );

    pub fn CFRunLoopSourceCreate(
        allocator: CFAllocatorRef,
        order: CFIndex,
        context: *mut CFRunLoopSourceContext,
    ) -> CFRunLoopSourceRef;
    pub fn CFRunLoopAddSource(
        rl: CFRunLoopRef,
        source: CFRunLoopSourceRef,
        mode: CFRunLoopMode,
    );
    pub fn CFRunLoopSourceInvalidate(source: CFRunLoopSourceRef);
    pub fn CFRunLoopSourceSignal(source: CFRunLoopSourceRef);

    pub fn CFAbsoluteTimeGetCurrent() -> CFAbsoluteTime;
    pub fn CFRelease(cftype: *const c_void);
}

pub type Boolean = u8;
pub enum CFAllocator {}
pub type CFAllocatorRef = *mut CFAllocator;
pub enum CFRunLoop {}
pub type CFRunLoopRef = *mut CFRunLoop;
pub type CFRunLoopMode = CFStringRef;
pub enum CFRunLoopObserver {}
pub type CFRunLoopObserverRef = *mut CFRunLoopObserver;
pub enum CFRunLoopTimer {}
pub type CFRunLoopTimerRef = *mut CFRunLoopTimer;
pub enum CFRunLoopSource {}
pub type CFRunLoopSourceRef = *mut CFRunLoopSource;
pub enum CFString {}
pub type CFStringRef = *const CFString;

pub type CFHashCode = c_ulong;
pub type CFIndex = c_long;
pub type CFOptionFlags = c_ulong;
pub type CFRunLoopActivity = CFOptionFlags;

pub type CFAbsoluteTime = CFTimeInterval;
pub type CFTimeInterval = f64;

pub const kCFRunLoopEntry: CFRunLoopActivity = 0;
pub const kCFRunLoopBeforeWaiting: CFRunLoopActivity = 1 << 5;
pub const kCFRunLoopAfterWaiting: CFRunLoopActivity = 1 << 6;
pub const kCFRunLoopExit: CFRunLoopActivity = 1 << 7;

pub type CFRunLoopObserverCallBack = extern "C" fn(
    observer: CFRunLoopObserverRef,
    activity: CFRunLoopActivity,
    info: *mut c_void,
);
pub type CFRunLoopTimerCallBack = extern "C" fn(
    timer: CFRunLoopTimerRef,
    info: *mut c_void
);

pub enum CFRunLoopObserverContext {}
pub enum CFRunLoopTimerContext {}

#[repr(C)]
pub struct CFRunLoopSourceContext {
    pub version: CFIndex,
    pub info: *mut c_void,
    pub retain: extern "C" fn(*const c_void) -> *const c_void,
    pub release: extern "C" fn(*const c_void),
    pub copyDescription: extern "C" fn(*const c_void) -> CFStringRef,
    pub equal: extern "C" fn(*const c_void, *const c_void) -> Boolean,
    pub hash: extern "C" fn(*const c_void) -> CFHashCode,
    pub schedule: extern "C" fn(*mut c_void, CFRunLoopRef, CFRunLoopMode),
    pub cancel: extern "C" fn(*mut c_void, CFRunLoopRef, CFRunLoopMode),
    pub perform: extern "C" fn(*mut c_void),
}

pub trait NSString: Sized {
    unsafe fn alloc(_: Self) -> id {
        msg_send![class!(NSString), alloc]
    }

    unsafe fn initWithUTF8String_(self, c_string: *const c_char) -> id;
    unsafe fn stringByAppendingString_(self, other: id) -> id;
    unsafe fn init_str(self, string: &str) -> Self;
    unsafe fn UTF8String(self) -> *const c_char;
}

impl NSString for id {
    unsafe fn initWithUTF8String_(self, c_string: *const c_char) -> id {
        msg_send![self, initWithUTF8String:c_string as id]
    }

    unsafe fn stringByAppendingString_(self, other: id) -> id {
        msg_send![self, stringByAppendingString:other]
    }

    unsafe fn init_str(self, string: &str) -> id {
        let cstring = CString::new(string).unwrap();
        self.initWithUTF8String_(cstring.as_ptr())
    }

    unsafe fn UTF8String(self) -> *const c_char {
        msg_send![self, UTF8String]
    }
}
