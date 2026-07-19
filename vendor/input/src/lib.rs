//! # Libinput bindings for rust
//!
//! These bindings closely follow libinput's concepts and it's original API.
//! Please refer to the [libinput documentation](https://wayland.freedesktop.org/libinput/doc/latest/)
//! to understand the general structure and concepts.
//!
//! ## Differences to the C-Library:
//!
//! - Refcounting does not need to be done manually. Just call `clone` when you need an additional reference.
//! - Libinput logging cannot (currently) not be customized.
//!
//! ## Userdata handling
//!
//! Multiple types in the libinput library allow to attach a pointer of an arbitrary type, so called `userdata`.
//! Using this data is unsafe as there is no way to find out what type is stored in the libinput struct.
//! Additionally multiple references to the same libinput object may exist and userdata may be shared mutably.
//!
//! This is why using and setting userdata is an unsafe operation (except when creating an object).
//!
//! If you heavily rely on userdata, you should always stored them wrapped in a `Mutex` and use the same
//! type for every userdata access to further simplify usage.
//!
//! You need to be especially cautious when initializing libinput types from raw pointers, you obtained
//! from other libraries which may set their own userdata. If accessing their userdata make sure no shared
//! mutable access may happen and don't store something else instead, if the library does not explicitly
//! allow this.
//!
//! Generally usage of this api is error-prone and discouraged if not needed.
//!
//! ## Getting started
//! To get started check out the [`Libinput` struct](./struct.Libinput.html).
//!
//! Here's a small example that prints all events:
//!
//! ```
//! use input::{Libinput, LibinputInterface};
//! use std::fs::{File, OpenOptions};
//! use std::os::unix::{fs::OpenOptionsExt, io::OwnedFd};
//! use std::path::Path;
//!
//! extern crate libc;
//! use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
//!
//! struct Interface;
//!
//! impl LibinputInterface for Interface {
//!     fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
//!         OpenOptions::new()
//!             .custom_flags(flags)
//!             .read((flags & O_ACCMODE == O_RDONLY) | (flags & O_ACCMODE == O_RDWR))
//!             .write((flags & O_ACCMODE == O_WRONLY) | (flags & O_ACCMODE == O_RDWR))
//!             .open(path)
//!             .map(|file| file.into())
//!             .map_err(|err| err.raw_os_error().unwrap())
//!     }
//!     fn close_restricted(&mut self, fd: OwnedFd) {
//!         unsafe {
//!             File::from(fd);
//!         }
//!     }
//! }
//!
//! fn main() {
//! #   // Preventing infinite execution (in particular on CI)
//! #   std::thread::spawn(|| {
//! #       std::thread::sleep(std::time::Duration::from_secs(5));
//! #       std::process::exit(0);
//! #   });
//! #
//!     let mut input = Libinput::new_with_udev(Interface);
//!     input.udev_assign_seat("seat0").unwrap();
//!     loop {
//!         input.dispatch().unwrap();
//!         for event in &mut input {
//!             println!("Got event: {:?}", event);
//!         }
//!     }
//! }
//! ```

#![deny(missing_docs)]

/// Unsafe raw C API.
pub mod ffi {
    pub use input_sys::*;
}

/// Trait for types that allow to optain the underlying raw libinput pointer.
pub trait AsRaw<T> {
    /// Receive a raw pointer representing this type.
    fn as_raw(&self) -> *const T;

    #[doc(hidden)]
    fn as_raw_mut(&self) -> *mut T {
        self.as_raw() as *mut _
    }
}

/// Trait to receive the underlying context
pub trait Context {
    /// Returns the underlying libinput context
    fn context(&self) -> &Libinput;
}

/// Trait for types that allow to be initialized from a raw pointer
pub trait FromRaw<T> {
    #[doc(hidden)]
    unsafe fn try_from_raw(ffi: *mut T, context: &context::Libinput) -> Option<Self>
    where
        Self: Sized;

    /// Create a new instance of this type from a raw pointer and it's context.
    /// If the type of the struct is a valid libinput type, but is unknown to this library, it panics instead.
    ///
    /// ## Warning
    ///
    /// If you make use of [`Userdata`](./trait.Userdata.html) make sure you use the correct types
    /// to allow receiving the set userdata. When dealing with raw pointers initialized by other
    /// libraries this must be done extra carefully to select a correct representation.
    ///
    /// If unsure using `()` is always a safe option..
    ///
    /// # Safety
    ///
    /// If the pointer is pointing to a different struct, invalid memory or `NULL` the returned
    /// struct may panic on use or cause other undefined behavior.
    unsafe fn from_raw(ffi: *mut T, context: &context::Libinput) -> Self;
}

macro_rules! ffi_ref_struct {
    ($(#[$attr:meta])* struct $struct_name:ident, $ffi_name:path, $ref_fn:path, $unref_fn:path) => (
        #[derive(Eq)]
        $(#[$attr])*
        pub struct $struct_name
        {
            ffi: *mut $ffi_name,
            context: $crate::context::Libinput,
        }

        impl ::std::fmt::Debug for $struct_name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                write!(f, "{} @{:p}", stringify!($struct_name), self.as_raw())
            }
        }

        impl $crate::FromRaw<$ffi_name> for $struct_name
        {
            unsafe fn try_from_raw(ffi: *mut $ffi_name, context: &$crate::Libinput) -> Option<Self> {
                Some(Self::from_raw(ffi, context))
            }
            unsafe fn from_raw(ffi: *mut $ffi_name, context: &$crate::Libinput) -> Self {
                $struct_name {
                    ffi: $ref_fn(ffi),
                    context: context.clone(),
                }
            }
        }

        impl $crate::AsRaw<$ffi_name> for $struct_name
        {
            fn as_raw(&self) -> *const $ffi_name {
                self.ffi as *const _
            }
        }

        impl $crate::Context for $struct_name
        {
            fn context(&self) -> &$crate::Libinput {
                &self.context
            }
         }

        impl Clone for $struct_name {
            fn clone(&self) -> Self {
                unsafe { $struct_name::from_raw(self.as_raw_mut(), &self.context) }
            }
        }

        impl Drop for $struct_name
        {
            fn drop(&mut self) {
                unsafe {
                    $unref_fn(self.ffi);
                }
            }
        }

        impl PartialEq for $struct_name {
            fn eq(&self, other: &Self) -> bool {
                self.as_raw() == other.as_raw()
            }
        }

        impl ::std::hash::Hash for $struct_name {
            fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
                self.as_raw().hash(state);
            }
        }
    )
}

macro_rules! ffi_func {
    ($(#[$attr:meta])* fn $name:ident, $ffi_fn:path, bool) => (
        $(#[$attr])*
        fn $name(&self) -> bool {
            unsafe { $ffi_fn(self.as_raw_mut()) != 0 }
        }
    );
    ($(#[$attr:meta])* pub fn $name:ident, $ffi_fn:path, bool) => (
        $(#[$attr])*
        pub fn $name(&self) -> bool {
            unsafe { $ffi_fn(self.as_raw_mut()) != 0 }
        }
    );
    ($(#[$attr:meta])* fn $name:ident, $ffi_fn:path, $return_type:ty) => (
        $(#[$attr])*
        fn $name(&self) -> $return_type {
            unsafe { $ffi_fn(self.as_raw_mut()) as $return_type }
        }
    );
    ($(#[$attr:meta])* pub fn $name:ident, $ffi_fn:path, $return_type:ty) => (
        $(#[$attr])*
        pub fn $name(&self) -> $return_type {
            unsafe { $ffi_fn(self.as_raw_mut()) as $return_type }
        }
    );
}

mod context;
mod device;
pub mod event;
mod seat;

pub use context::*;
pub use device::*;
pub use event::Event;
pub use seat::*;

#[cfg(feature = "libinput_1_23")]
mod accel_config;
#[cfg(feature = "libinput_1_23")]
pub use accel_config::*;
