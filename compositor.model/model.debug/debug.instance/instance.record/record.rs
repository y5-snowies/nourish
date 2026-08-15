pub use compositor_model_debug_instance_channel::{
    SENDER, START, abort, install_sender, push, runtime_enabled, set_enabled_mask, set_start,
    since_start,
};

pub use compositor_model_debug_instance_level::{Instance, Level, Record, parse_levels};

/// OPTIONAL: declare a static `DEBUG` instance for this crate. No longer required — the
/// level macros derive the crate name from `env!("CARGO_PKG_NAME")`. Kept for back-compat.
#[macro_export]
macro_rules! instance {
    () => {
        pub static DEBUG: $crate::Instance = $crate::Instance::new(env!("CARGO_PKG_NAME"));
    };
}

/// Compile-time current-function path (`module::path::function`) via the stable
/// type-name trick. Zero runtime cost, returns `&'static str`.
#[macro_export]
macro_rules! function {
    () => {{
        fn __y5_f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            ::core::any::type_name::<T>()
        }
        let name = type_name_of(__y5_f);
        // strip the trailing "::__y5_f"
        &name[..name.len() - 8]
    }};
}

/// Internal: build + push a record. Not called directly — use the level macros.
#[macro_export]
macro_rules! __emit {
    ($level:expr, $($arg:tt)*) => {{
        if $crate::runtime_enabled($level) {
            // `env!("CARGO_PKG_NAME")` expands in the CALLER crate → its name, &'static.
            $crate::push($crate::Record::with(
                $level,
                env!("CARGO_PKG_NAME"),
                $crate::function!(),
                ::std::format!($($arg)*),
            ));
        }
    }};
}

// For each level: a real macro when the feature is on, a no-op (args not even evaluated)
// when off — TRUE stripping. The `#[cfg]` is evaluated against THIS crate's features.

#[cfg(feature = "error")]
#[macro_export]
macro_rules! error { ($($arg:tt)*) => {{ $crate::__emit!($crate::Level::Error, $($arg)*); }}; }

#[cfg(not(feature = "error"))]
#[macro_export]
macro_rules! error { ($($arg:tt)*) => {{}}; }

#[cfg(feature = "warn")]
#[macro_export]
macro_rules! warn { ($($arg:tt)*) => {{ $crate::__emit!($crate::Level::Warn, $($arg)*); }}; }

#[cfg(not(feature = "warn"))]
#[macro_export]
macro_rules! warn { ($($arg:tt)*) => {{}}; }

#[cfg(feature = "info")]
#[macro_export]
macro_rules! info { ($($arg:tt)*) => {{ $crate::__emit!($crate::Level::Info, $($arg)*); }}; }

#[cfg(not(feature = "info"))]
#[macro_export]
macro_rules! info { ($($arg:tt)*) => {{}}; }

#[cfg(feature = "trace")]
#[macro_export]
macro_rules! trace { ($($arg:tt)*) => {{ $crate::__emit!($crate::Level::Trace, $($arg)*); }}; }

#[cfg(not(feature = "trace"))]
#[macro_export]
macro_rules! trace { ($($arg:tt)*) => {{}}; }

/// Like `panic!`, but also emits an Error-level log record first. Diverges. Always
/// active — independent of the level features and `COMPOSITOR_LOG_LEVEL`.
#[macro_export]
macro_rules! abort {
    () => {
        $crate::abort(env!("CARGO_PKG_NAME"), $crate::function!(), ::std::string::String::from("explicit abort"))
    };
    ($($arg:tt)*) => {
        $crate::abort(env!("CARGO_PKG_NAME"), $crate::function!(), ::std::format!($($arg)*))
    };
}
