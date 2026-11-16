#[cfg(not(unix))]
pub fn run_window(_url: &str) -> Result<(), String> {
    Err("the native GTK/WebKit window is only available on Unix".into())
}

#[cfg(not(unix))]
pub fn set_fullscreen(_enabled: bool) -> bool {
    false
}

#[cfg(unix)]
pub use unix::{run_window, set_fullscreen};

#[cfg(unix)]
mod unix {
    use std::ffi::{c_char, c_int, c_ulong, c_void, CString};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::OnceLock;

    type Window = *mut c_void;
    type GtkInitCheck = unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) -> c_int;
    type GtkWindowNew = unsafe extern "C" fn(c_int) -> Window;
    type GtkWindowSetTitle = unsafe extern "C" fn(Window, *const c_char);
    type GtkWindowSetDefaultSize = unsafe extern "C" fn(Window, c_int, c_int);
    type GtkWidgetSetSizeRequest = unsafe extern "C" fn(Window, c_int, c_int);
    type GtkContainerAdd = unsafe extern "C" fn(Window, Window);
    type GtkWidgetShowAll = unsafe extern "C" fn(Window);
    type GtkWindowFullscreen = unsafe extern "C" fn(Window);
    type GtkMain = unsafe extern "C" fn();
    type WebKitWebViewNew = unsafe extern "C" fn() -> Window;
    type WebKitWebViewLoadUri = unsafe extern "C" fn(Window, *const c_char);
    type DestroyCallback = unsafe extern "C" fn(Window, Window);
    type ClosureNotify = unsafe extern "C" fn(Window, Window);
    type GSignalConnectData = unsafe extern "C" fn(
        Window,
        *const c_char,
        Option<DestroyCallback>,
        Window,
        Option<ClosureNotify>,
        c_int,
    ) -> c_ulong;
    type IdleCallback = unsafe extern "C" fn(Window) -> c_int;
    type GIdleAdd = unsafe extern "C" fn(Option<IdleCallback>, Window) -> u32;

    #[derive(Clone, Copy)]
    struct Api {
        gtk_init_check: GtkInitCheck,
        gtk_window_new: GtkWindowNew,
        gtk_window_set_title: GtkWindowSetTitle,
        gtk_window_set_default_size: GtkWindowSetDefaultSize,
        gtk_widget_set_size_request: GtkWidgetSetSizeRequest,
        gtk_container_add: GtkContainerAdd,
        gtk_widget_show_all: GtkWidgetShowAll,
        gtk_window_fullscreen: GtkWindowFullscreen,
        gtk_window_unfullscreen: GtkWindowFullscreen,
        gtk_main: GtkMain,
        gtk_main_quit: GtkMain,
        webkit_web_view_new: WebKitWebViewNew,
        webkit_web_view_load_uri: WebKitWebViewLoadUri,
        g_signal_connect_data: GSignalConnectData,
        g_idle_add: GIdleAdd,
    }

    static API: OnceLock<Api> = OnceLock::new();
    static WINDOW: AtomicUsize = AtomicUsize::new(0);
    static FULLSCREEN: AtomicBool = AtomicBool::new(false);

    const RTLD_NOW: c_int = 2;
    const RTLD_GLOBAL: c_int = 0x100;
    const GTK_WINDOW_TOPLEVEL: c_int = 0;

    #[link(name = "dl")]
    extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    pub fn run_window(url: &str) -> Result<(), String> {
        let api = load_api()?;
        let title = CString::new("ConnectK").unwrap();
        let uri = CString::new(url).map_err(|_| "invalid local GUI URL".to_string())?;
        let destroy = CString::new("destroy").unwrap();

        unsafe {
            if (api.gtk_init_check)(std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
                return Err("GTK could not initialize a display".into());
            }
            let window = (api.gtk_window_new)(GTK_WINDOW_TOPLEVEL);
            if window.is_null() {
                return Err("GTK could not create a window".into());
            }
            WINDOW.store(window as usize, Ordering::SeqCst);
            (api.gtk_window_set_title)(window, title.as_ptr());
            (api.gtk_window_set_default_size)(window, 1140, 740);
            (api.gtk_widget_set_size_request)(window, 860, 600);

            let view = (api.webkit_web_view_new)();
            if view.is_null() {
                WINDOW.store(0, Ordering::SeqCst);
                return Err("WebKit could not create a web view".into());
            }
            (api.gtk_container_add)(window, view);
            (api.webkit_web_view_load_uri)(view, uri.as_ptr());
            (api.g_signal_connect_data)(
                window,
                destroy.as_ptr(),
                Some(on_destroy),
                std::ptr::null_mut(),
                None,
                0,
            );
            (api.gtk_widget_show_all)(window);
            (api.gtk_main)();
            WINDOW.store(0, Ordering::SeqCst);
        }
        Ok(())
    }

    pub fn set_fullscreen(enabled: bool) -> bool {
        let Some(api) = API.get() else {
            return false;
        };
        if WINDOW.load(Ordering::SeqCst) == 0 {
            return false;
        }
        FULLSCREEN.store(enabled, Ordering::SeqCst);
        unsafe {
            (api.g_idle_add)(Some(apply_fullscreen), std::ptr::null_mut());
        }
        true
    }

    unsafe extern "C" fn on_destroy(_widget: Window, _data: Window) {
        if let Some(api) = API.get() {
            unsafe {
                (api.gtk_main_quit)();
            }
        }
    }

    unsafe extern "C" fn apply_fullscreen(_data: Window) -> c_int {
        let window = WINDOW.load(Ordering::SeqCst) as Window;
        if !window.is_null() {
            if let Some(api) = API.get() {
                unsafe {
                    if FULLSCREEN.load(Ordering::SeqCst) {
                        (api.gtk_window_fullscreen)(window);
                    } else {
                        (api.gtk_window_unfullscreen)(window);
                    }
                }
            }
        }
        0
    }

    fn load_api() -> Result<&'static Api, String> {
        if let Some(api) = API.get() {
            return Ok(api);
        }
        let gtk = load_library(&["libgtk-3.so.0", "libgtk-3.so"])?;
        let gobject = load_library(&["libgobject-2.0.so.0", "libgobject-2.0.so"])?;
        let glib = load_library(&["libglib-2.0.so.0", "libglib-2.0.so"])?;
        let webkit = load_library(&[
            "libwebkit2gtk-4.1.so.0",
            "libwebkit2gtk-4.1.so",
            "libwebkit2gtk-4.0.so.0",
            "libwebkit2gtk-4.0.so",
        ])?;

        let api = unsafe {
            Api {
                gtk_init_check: symbol(gtk, "gtk_init_check")?,
                gtk_window_new: symbol(gtk, "gtk_window_new")?,
                gtk_window_set_title: symbol(gtk, "gtk_window_set_title")?,
                gtk_window_set_default_size: symbol(gtk, "gtk_window_set_default_size")?,
                gtk_widget_set_size_request: symbol(gtk, "gtk_widget_set_size_request")?,
                gtk_container_add: symbol(gtk, "gtk_container_add")?,
                gtk_widget_show_all: symbol(gtk, "gtk_widget_show_all")?,
                gtk_window_fullscreen: symbol(gtk, "gtk_window_fullscreen")?,
                gtk_window_unfullscreen: symbol(gtk, "gtk_window_unfullscreen")?,
                gtk_main: symbol(gtk, "gtk_main")?,
                gtk_main_quit: symbol(gtk, "gtk_main_quit")?,
                webkit_web_view_new: symbol(webkit, "webkit_web_view_new")?,
                webkit_web_view_load_uri: symbol(webkit, "webkit_web_view_load_uri")?,
                g_signal_connect_data: symbol(gobject, "g_signal_connect_data")?,
                g_idle_add: symbol(glib, "g_idle_add")?,
            }
        };
        let _ = API.set(api);
        API.get()
            .ok_or_else(|| "native GUI API initialization failed".to_string())
    }

    fn load_library(names: &[&str]) -> Result<*mut c_void, String> {
        for name in names {
            let Ok(c_name) = CString::new(*name) else {
                continue;
            };
            let handle = unsafe { dlopen(c_name.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
            if !handle.is_null() {
                return Ok(handle);
            }
        }
        Err(format!(
            "could not load a native GUI library (tried {})",
            names.join(", ")
        ))
    }

    unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &str) -> Result<T, String> {
        let c_name = CString::new(name).unwrap();
        let pointer = unsafe { dlsym(handle, c_name.as_ptr()) };
        if pointer.is_null() {
            return Err(format!("native GUI symbol not found: {name}"));
        }
        Ok(unsafe { std::mem::transmute_copy(&pointer) })
    }
}
