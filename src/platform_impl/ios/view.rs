use std::collections::HashMap;
use std::mem;

use objc::declare::ClassDecl;
use objc::runtime::{BOOL, Class, NO, Object, Sel, YES};

use event::{
    DeviceId as RootDeviceId,
    Event,
    Touch,
    TouchPhase,
    WindowEvent
};
use platform::ios::MonitorHandleExtIOS;
use window::WindowId as RootWindowId;

use platform_impl::platform::DeviceId;
use platform_impl::platform::event_loop::{self, RawEvent};
use platform_impl::platform::ffi::{
    id,
    nil,
    CGFloat,
    CGPoint,
    CGRect,
    CGSize,
    UITouchPhase,
};
use platform_impl::platform::monitor;
use platform_impl::platform::shared::{ConfiguredWindow, Running};
use platform_impl::platform::window::WindowId;

unsafe fn get_view_class(config: &ConfiguredWindow) -> &'static Class {
    static mut CLASSES: Option<HashMap<*const Class, &'static Class>> = None;
    static mut ID: usize = 0;
    
    if CLASSES.is_none() {
        CLASSES = Some(HashMap::default());
    }

    let classes = CLASSES.as_mut().unwrap();

    classes.entry(&(config.platform_attributes.root_view_class as _)).or_insert_with(move || {
        let uiview_class = class!(UIView);
        let root_view_class = config.platform_attributes.root_view_class;
        let is_uiview: BOOL = msg_send![root_view_class, isSubclassOfClass:uiview_class];
        assert_eq!(is_uiview, YES, "`root_view_class` must inherit from `UIView`");

        extern fn draw_rect(object: &Object, _: Sel, rect: CGRect) {
            unsafe {
                event_loop::process_erased_event(Event::WindowEvent {
                    window_id: RootWindowId(WindowId),
                    event: WindowEvent::RedrawRequested,
                });
                let superclass: id = msg_send![object, superclass];
                let () = msg_send![super(object, mem::transmute(superclass)), drawRect: rect];
            }
        }

        extern fn layout_subviews(object: &Object, _: Sel) {
            unsafe {
                let bounds: CGRect = msg_send![object, bounds];
                let size = crate::dpi::LogicalSize {
                    width: bounds.size.width,
                    height: bounds.size.height,
                };
                event_loop::process_erased_event(Event::WindowEvent {
                    window_id: RootWindowId(WindowId),
                    event: WindowEvent::Resized(size),
                });
                let superclass: id = msg_send![object, superclass];
                let () = msg_send![super(object, mem::transmute(superclass)), layoutSubviews];
            }
        }

        let mut decl = ClassDecl::new(&format!("WinitUIView{}", ID), root_view_class)
            .expect("Failed to declare class `WinitUIView`");
        ID += 1;
        decl.add_method(sel!(drawRect:),
                        draw_rect as extern fn(&Object, Sel, CGRect));
        decl.add_method(sel!(layoutSubviews),
                        layout_subviews as extern fn(&Object, Sel));
        decl.register()
    })
}

unsafe fn get_view_controller_class() -> &'static Class {
    static mut CLASS: Option<&'static Class> = None;
    if CLASS.is_none() {
        let uiviewcontroller_class = class!(UIViewController);

        extern fn set_prefers_status_bar_hidden(object: &mut Object, _: Sel, hidden: BOOL) {
            unsafe {
                object.set_ivar::<BOOL>("_prefers_status_bar_hidden", hidden);
                let () = msg_send![object, setNeedsStatusBarAppearanceUpdate];
            }
        }

        extern fn prefers_status_bar_hidden(object: &Object, _: Sel) -> BOOL {
            unsafe {
                *object.get_ivar::<BOOL>("_prefers_status_bar_hidden")
            }
        }

        let mut decl = ClassDecl::new("WinitUIViewController", uiviewcontroller_class)
            .expect("Failed to declare class `WinitUIViewController`");
        decl.add_ivar::<BOOL>("_prefers_status_bar_hidden");
        decl.add_method(sel!(setPrefersStatusBarHidden:),
                        set_prefers_status_bar_hidden as extern fn(&mut Object, Sel, BOOL));
        decl.add_method(sel!(prefersStatusBarHidden),
                        prefers_status_bar_hidden as extern fn(&Object, Sel) -> BOOL);
        CLASS = Some(decl.register());
    }
    CLASS.unwrap()
}

unsafe fn get_window_class() -> &'static Class {
    static mut CLASS: Option<&'static Class> = None;
    if CLASS.is_none() {
        let uiwindow_class = class!(UIWindow);

        extern fn become_key_window(_: &Object, _: Sel) {
            // TODO: Focused(true) event?
        }

        extern fn resign_key_window(_: &Object, _: Sel) {
            // TODO: Focused(false) event?
        }

        let mut decl = ClassDecl::new("WinitUIWindow", uiwindow_class)
            .expect("Failed to declare class `WinitUIWindow`");
        decl.add_method(sel!(becomeKeyWindow),
                        become_key_window as extern fn(&Object, Sel));
        decl.add_method(sel!(resignKeyWindow),
                        resign_key_window as extern fn(&Object, Sel));
        CLASS = Some(decl.register());
    }
    CLASS.unwrap()
}

unsafe fn create_view(config: &ConfiguredWindow, bounds: CGRect) -> id {
    let class = get_view_class(config);

    let view: id = msg_send![class, alloc];
    assert!(!view.is_null(), "Failed to create `UIView` instance");
    let view: id = msg_send![view, initWithFrame:bounds];
    assert!(!view.is_null(), "Failed to initialize `UIView` instance");
    if config.window_attributes.multitouch {
        let () = msg_send![view, setMultipleTouchEnabled:YES];
    }

    view
}

unsafe fn create_view_controller(config: &ConfiguredWindow, view: id) -> id {
    let class = get_view_controller_class();

    let view_controller: id = msg_send![class, alloc];
    assert!(!view_controller.is_null(), "Failed to create `UIViewController` instance");
    let view_controller: id = msg_send![view_controller, init];
    assert!(!view_controller.is_null(), "Failed to initialize `UIViewController` instance");
    let status_bar_hidden = if config.platform_attributes.status_bar_hidden {
        YES
    } else {
        NO
    };
    let () = msg_send![view_controller, setPrefersStatusBarHidden:status_bar_hidden];
    let () = msg_send![view_controller, setView:view];
    view_controller
}

unsafe fn create_window(config: &ConfiguredWindow, bounds: CGRect, view_controller: id) -> id {
    let class = get_window_class();

    let window: id = msg_send![class, alloc];
    assert!(!window.is_null(), "Failed to create `UIWindow` instance");
    let window: id = msg_send![window, initWithFrame:bounds];
    assert!(!window.is_null(), "Failed to initialize `UIWindow` instance");
    let () = msg_send![window, setRootViewController:view_controller];
    if let Some(content_scale_factor) = config.platform_attributes.content_scale_factor {
        let () = msg_send![window, setContentScaleFactor:content_scale_factor as CGFloat];
    }
    if let &Some(ref monitor) = &config.window_attributes.fullscreen {
        let () = msg_send![window, setScreen:monitor.get_uiscreen()];
    }

    window
}

pub fn create_delegate_class() {
    extern fn did_finish_launching(_: &mut Object, _: Sel, _: id, _: id) -> BOOL {
        unsafe {
            event_loop::run(move |config| {
                let screen = config.window_attributes.fullscreen
                    .as_ref()
                    .map(|screen| screen.get_uiscreen() as _)
                    .unwrap_or_else(|| monitor::main_uiscreen().get_uiscreen());
                let bounds: CGRect = msg_send![screen, bounds];

                let bounds = match config.window_attributes.dimensions {
                    Some(dim) => CGRect {
                        origin: bounds.origin,
                        size: CGSize { width: dim.width, height: dim.height },
                    },
                    None => bounds,
                };

                let view = create_view(config, bounds.clone());
                let view_controller = create_view_controller(config, view);
                let window = create_window(config, bounds, view_controller);
                let () = msg_send![window, makeKeyAndVisible];

                Running {
                    view,
                    window,
                    view_controller,
                }
            });

            event_loop::process_erased_event(RawEvent::Init);
        }
        YES
    }

    extern fn did_become_active(_: &Object, _: Sel, _: id) {
        unsafe {
            event_loop::process_erased_event(Event::WindowEvent {
                window_id: RootWindowId(WindowId),
                event: WindowEvent::Focused(true),
            })
        }
    }

    extern fn will_resign_active(_: &Object, _: Sel, _: id) {
        unsafe {
            event_loop::process_erased_event(Event::WindowEvent {
                window_id: RootWindowId(WindowId),
                event: WindowEvent::Focused(false),
            })
        }
    }

    extern fn will_enter_foreground(_: &Object, _: Sel, _: id) {
        unsafe {
            event_loop::process_erased_event(Event::Suspended(false))
        }
    }

    extern fn did_enter_background(_: &Object, _: Sel, _: id) {
        unsafe {
            event_loop::process_erased_event(Event::Suspended(true))
        }
    }

    extern fn will_terminate(_: &Object, _: Sel, _: id) {
        unsafe {
            event_loop::process_erased_event(Event::WindowEvent {
                window_id: RootWindowId(WindowId),
                event: WindowEvent::Destroyed,
            });
            event_loop::process_erased_event(Event::LoopDestroyed);
        }
    }

    extern fn handle_touches(_: &Object, _: Sel, touches: id, _:id) {
        unsafe {
            let touches_enum: id = msg_send![touches, objectEnumerator];
            loop {
                let touch: id = msg_send![touches_enum, nextObject];
                if touch == nil {
                    break
                }
                let location: CGPoint = msg_send![touch, locationInView:nil];
                let touch_id = touch as u64;
                let phase: UITouchPhase = msg_send![touch, phase];
                let phase = match phase {
                    UITouchPhase::Began => TouchPhase::Started,
                    UITouchPhase::Moved => TouchPhase::Moved,
                    // 2 is UITouchPhase::Stationary and is not expected here
                    UITouchPhase::Ended => TouchPhase::Ended,
                    UITouchPhase::Cancelled => TouchPhase::Cancelled,
                    _ => panic!("unexpected touch phase: {:?}", phase as i32),
                };

                event_loop::process_erased_event(Event::WindowEvent {
                    window_id: RootWindowId(WindowId),
                    event: WindowEvent::Touch(Touch {
                        device_id: RootDeviceId(DeviceId),
                        id: touch_id,
                        location: (location.x as f64, location.y as f64).into(),
                        phase,
                    }),
                });
            }
        }
    }

    let ui_responder = class!(UIResponder);
    let mut decl = ClassDecl::new("AppDelegate", ui_responder).expect("Failed to declare class `AppDelegate`");

    unsafe {
        decl.add_method(sel!(application:didFinishLaunchingWithOptions:),
                        did_finish_launching as extern fn(&mut Object, Sel, id, id) -> BOOL);

        decl.add_method(sel!(applicationDidBecomeActive:),
                        did_become_active as extern fn(&Object, Sel, id));

        decl.add_method(sel!(applicationWillResignActive:),
                        will_resign_active as extern fn(&Object, Sel, id));

        decl.add_method(sel!(applicationWillEnterForeground:),
                        will_enter_foreground as extern fn(&Object, Sel, id));

        decl.add_method(sel!(applicationDidEnterBackground:),
                        did_enter_background as extern fn(&Object, Sel, id));

        decl.add_method(sel!(applicationWillTerminate:),
                        will_terminate as extern fn(&Object, Sel, id));


        decl.add_method(sel!(touchesBegan:withEvent:),
                        handle_touches as extern fn(this: &Object, _: Sel, _: id, _:id));

        decl.add_method(sel!(touchesMoved:withEvent:),
                        handle_touches as extern fn(this: &Object, _: Sel, _: id, _:id));

        decl.add_method(sel!(touchesEnded:withEvent:),
                        handle_touches as extern fn(this: &Object, _: Sel, _: id, _:id));

        decl.add_method(sel!(touchesCancelled:withEvent:),
                        handle_touches as extern fn(this: &Object, _: Sel, _: id, _:id));

        decl.register();
    }
}
