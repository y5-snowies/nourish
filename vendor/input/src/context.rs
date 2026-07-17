// TODO Error type instead of `Result<_, ()>`
#![allow(clippy::result_unit_err)]

use crate::{ffi, AsRaw, Device, Event, FromRaw};
use std::{
    ffi::{CStr, CString},
    io::{Error as IoError, Result as IoResult},
    iter::Iterator,
    os::unix::io::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd},
    path::Path,
    rc::Rc,
};
#[cfg(feature = "udev")]
use udev::ffi as udev;

/// libinput does not open file descriptors to devices directly,
/// instead `open_restricted` and `close_restricted` are called for
/// each path that must be opened.
///
/// Implementations are of course permitted to just use `open` and
/// `close` respectively, but doing so would require root permissions
/// to open devices. This interface provides an api agnostic way to
/// use ConsoleKit or similar endpoints to open devices without
/// direct priviledge escalation.
pub trait LibinputInterface {
    /// Open the device at the given path with the flags provided and
    /// return the fd.
    ///
    /// ## Paramater
    /// - `path` - The device path to open
    /// - `flags` Flags as defined by open(2)
    ///
    /// ## Returns
    /// The file descriptor, or a negative errno on failure.
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32>;

    /// Close the file descriptor.
    ///
    /// ## Parameter
    /// `fd` - The file descriptor to close
    fn close_restricted(&mut self, fd: OwnedFd);
}

unsafe extern "C" fn open_restricted<I: LibinputInterface + 'static>(
    path: *const libc::c_char,
    flags: libc::c_int,
    user_data: *mut libc::c_void,
) -> libc::c_int {
    use std::borrow::Cow;

    if let Some(interface) = (user_data as *mut I).as_mut() {
        let path_str = CStr::from_ptr(path).to_string_lossy();
        let res = match path_str {
            Cow::Borrowed(string) => interface.open_restricted(Path::new(string), flags),
            Cow::Owned(string) => interface.open_restricted(Path::new(&string), flags),
        };
        match res {
            Ok(fd) => fd.into_raw_fd(),
            Err(errno) => {
                if errno > 0 {
                    -errno
                } else {
                    errno
                }
            }
        }
    } else {
        -1
    }
}

unsafe extern "C" fn close_restricted<I: LibinputInterface + 'static>(
    fd: libc::c_int,
    user_data: *mut libc::c_void,
) {
    if let Some(interface) = (user_data as *mut I).as_mut() {
        interface.close_restricted(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

/// Libinput context
///
/// Contexts can be used to track input devices and receive events from them.
/// You can use either `new_from_udev` to create a context tracking all devices on a specific seat,
/// or use `new_from_path` to track input devices manually.
///
/// Either way you then have to use `dispatch()` and `next()` (provided by the `Iterator` trait) to
/// receive events.
pub struct Libinput {
    ffi: *mut ffi::libinput,
    _interface: Option<Rc<dyn LibinputInterface + 'static>>,
}

impl ::std::fmt::Debug for Libinput {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        write!(f, "Libinput @{:p}", self.as_raw())
    }
}

impl AsRaw<ffi::libinput> for Libinput {
    fn as_raw(&self) -> *const ffi::libinput {
        self.ffi as *const _
    }
}

impl Clone for Libinput {
    fn clone(&self) -> Self {
        Libinput {
            ffi: unsafe { ffi::libinput_ref(self.as_raw_mut()) },
            _interface: self._interface.clone(),
        }
    }
}

impl Drop for Libinput {
    fn drop(&mut self) {
        unsafe {
            ffi::libinput_unref(self.ffi);
        }
    }
}

impl PartialEq for Libinput {
    fn eq(&self, other: &Self) -> bool {
        self.as_raw() == other.as_raw()
    }
}

impl Eq for Libinput {}

impl ::std::hash::Hash for Libinput {
    fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
        self.as_raw().hash(state);
    }
}

impl Iterator for Libinput {
    type Item = Event;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let ptr = unsafe { ffi::libinput_get_event(self.as_raw_mut()) };
            if ptr.is_null() {
                return None;
            } else {
                match unsafe { Event::try_from_raw(ptr, self) } {
                    Some(x) => return Some(x),
                    None => {
                        #[cfg(feature = "log")]
                        log::warn!("Skipping unknown event: {}", unsafe {
                            ffi::libinput_event_get_type(ptr)
                        });
                        continue;
                    }
                }
            }
        }
    }
}

impl Libinput {
    /// Create a new libinput context using a udev context.
    ///
    /// This context is inactive until `udev_assign_seat` is called.
    ///
    /// ## Arguments
    ///
    /// - interface - A `LibinputInterface` providing functions to open and close devices.
    /// - userdata - Optionally some userdata attached to the newly created context (see [`Userdata`](./trait.Userdata.html))
    /// - udev_context - Raw pointer to a valid udev context.
    ///
    /// # Safety
    ///
    /// This function is unsafe, because there is no way to verify that `udev_context` is indeed a valid udev context or even points to valid memory.
    #[cfg(feature = "udev")]
    pub fn new_with_udev<I: LibinputInterface + 'static>(interface: I) -> Libinput {
        let boxed_userdata = Rc::new(interface);
        let boxed_interface = Box::new(ffi::libinput_interface {
            open_restricted: Some(open_restricted::<I>),
            close_restricted: Some(close_restricted::<I>),
        });

        unsafe {
            let udev = udev::udev_new();
            let libinput = ffi::libinput_udev_create_context(
                Box::into_raw(boxed_interface),
                Rc::as_ptr(&boxed_userdata) as *mut _,
                udev as *mut input_sys::udev,
            );
            udev::udev_unref(udev);
            Libinput {
                ffi: libinput,
                _interface: Some(boxed_userdata as Rc<dyn LibinputInterface>),
            }
        }
    }

    /// Create a new libinput context that requires the caller to manually add or remove devices.
    ///
    /// The returned context is active, but will not yield any events
    /// until at least one device is added.
    ///
    /// Devices can be added and removed by calling `path_add_device` and `path_remove_device` respectively.
    ///
    /// ## Arguments
    ///
    /// - interface - A `LibinputInterface` providing functions to open and close devices.
    /// - userdata - Optionally some userdata attached to the newly created context (see [`Userdata`](./trait.Userdata.html))
    ///
    pub fn new_from_path<I: 'static + LibinputInterface>(interface: I) -> Libinput {
        let boxed_userdata = Rc::new(interface);
        let boxed_interface = Box::new(ffi::libinput_interface {
            open_restricted: Some(open_restricted::<I>),
            close_restricted: Some(close_restricted::<I>),
        });

        Libinput {
            ffi: unsafe {
                ffi::libinput_path_create_context(
                    Box::into_raw(boxed_interface),
                    Rc::as_ptr(&boxed_userdata) as *mut _,
                )
            },
            _interface: Some(boxed_userdata as Rc<dyn LibinputInterface>),
        }
    }

    /// Add a device to a libinput context initialized with
    /// `new_from_context`.
    ///
    /// If successful, the device will be added to the internal list
    /// and re-opened on `resume`. The device can be removed with
    /// `path_remove_device()`.
    ///
    /// If the device was successfully initialized, it is returned.
    ///
    /// ## Warning
    ///
    /// It is an application bug to call this function on a context
    /// initialized with `new_from_udev`.
    pub fn path_add_device(&mut self, path: &str) -> Option<Device> {
        let path = CString::new(path).expect("Device Path contained a null-byte");
        unsafe {
            let ptr = ffi::libinput_path_add_device(self.as_raw_mut(), path.as_ptr());
            if ptr.is_null() {
                None
            } else {
                Some(Device::from_raw(ptr, self))
            }
        }
    }

    /// Remove a device from a libinput context initialized with
    /// `new_from_path` and added to such a context with
    /// `path_add_device`.
    ///
    /// Events already processed from this input device are kept in
    /// the queue, the `DeviceRemovedEvent` event marks the end of
    /// events for this device.
    ///
    /// ## Warning
    ///
    /// It is an application bug to call this function on a context
    /// initialized with `new_from_udev`.
    pub fn path_remove_device(&mut self, device: Device) {
        unsafe { ffi::libinput_path_remove_device(device.as_raw_mut()) }
    }

    /// Assign a seat to this libinput context.
    ///
    /// New devices or the removal of existing devices will appear as
    /// events during `dispatch`.
    ///
    /// `udev_assign_seat` succeeds even if no input devices are
    /// currently available on this seat, or if devices are available
    /// but fail to open in `LibinputInterface::open_restricted`.
    ///
    /// Devices that do not have the minimum capabilities to be
    /// recognized as pointer, keyboard or touch device are ignored. /// Such devices and those that failed to open ignored until the
    /// next call to `resume`.
    ///
    /// ## Warning
    ///
    /// This function may only be called once per context.
    #[cfg(feature = "udev")]
    pub fn udev_assign_seat(&mut self, seat_id: &str) -> Result<(), ()> {
        let id = CString::new(seat_id).expect("Seat Id contained a null-byte");
        unsafe {
            match ffi::libinput_udev_assign_seat(self.as_raw_mut(), id.as_ptr()) {
                0 => Ok(()),
                -1 => Err(()),
                _ => unreachable!(),
            }
        }
    }

    ffi_func!(
    /// Suspend monitoring for new devices and close existing
    /// devices.
    ///
    /// This closes all open devices and terminates libinput but
    /// does keep the context valid to be resumed with `resume`.
    pub fn suspend, ffi::libinput_suspend, ());

    /// Resume a suspended libinput context.
    ///
    /// This re-enables device monitoring and adds existing devices.
    pub fn resume(&mut self) -> Result<(), ()> {
        unsafe {
            match ffi::libinput_resume(self.as_raw_mut()) {
                0 => Ok(()),
                -1 => Err(()),
                _ => unreachable!(),
            }
        }
    }

    /// Main event dispatchment function.
    ///
    /// Reads events of the file descriptors and processes them
    /// internally. Use `next` or any other function provided by the
    /// `Iterator` trait to retrieve the events until `None` is
    /// returned.
    ///
    /// Dispatching does not necessarily queue libinput events. This
    /// function should be called immediately once data is available
    /// on the file descriptor returned by `fd`. libinput has a number
    /// of timing-sensitive features (e.g. tap-to-click), any delay in
    /// calling `dispatch` may prevent these features from working
    /// correctly.
    pub fn dispatch(&mut self) -> IoResult<()> {
        unsafe {
            match ffi::libinput_dispatch(self.as_raw_mut()) {
                0 => Ok(()),
                x if x < 0 => Err(IoError::from_raw_os_error(-x)),
                _ => unreachable!(),
            }
        }
    }

    /// libinput keeps a single file descriptor for all events.
    ///
    /// Call into `dispatch` if any events become available on this fd.
    ///
    /// The most simple variant to check for available bytes is to use
    /// `nix::poll`:
    ///
    /// ```
    /// # extern crate libc;
    /// # extern crate rustix;
    /// #
    /// # use std::fs::{File, OpenOptions};
    /// # use std::os::unix::{fs::OpenOptionsExt, io::OwnedFd};
    /// # use std::path::Path;
    /// # use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
    /// #
    /// use input::{Libinput, LibinputInterface};
    /// use rustix::event::{poll, PollFlags, PollFd};
    ///
    /// # struct Interface;
    /// #
    /// # impl LibinputInterface for Interface {
    /// #     fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
    /// #         OpenOptions::new()
    /// #             .custom_flags(flags)
    /// #             .read((flags & O_ACCMODE == O_RDONLY) | (flags & O_ACCMODE == O_RDWR))
    /// #             .write((flags & O_ACCMODE == O_WRONLY) | (flags & O_ACCMODE == O_RDWR))
    /// #             .open(path)
    /// #             .map(|file| file.into())
    /// #             .map_err(|err| err.raw_os_error().unwrap())
    /// #     }
    /// #     fn close_restricted(&mut self, fd: OwnedFd) {
    /// #         unsafe {
    /// #             File::from(fd);
    /// #         }
    /// #     }
    /// # }
    /// #
    /// # // Preventing infinite execution (in particular on CI)
    /// # std::thread::spawn(|| {
    /// #     std::thread::sleep(std::time::Duration::from_secs(5));
    /// #     std::process::exit(0);
    /// # });
    /// #
    /// let mut input = Libinput::new_with_udev(Interface);
    /// input.udev_assign_seat("seat0").unwrap();
    ///
    /// while poll(&mut [PollFd::new(&input, PollFlags::IN)], None).is_ok() {
    ///     input.dispatch().unwrap();
    ///     for event in &mut input {
    ///         // do some processing...
    ///     }
    /// }
    /// ```
    ///
    /// For more complex operations you may wish to use other approches
    /// as event loops e.g. in the `wayland-server` or the `tokio`
    /// crates to wait for data to become available on this file
    /// descriptor.
    ///
    /// # Safety
    ///
    /// See [`AsRawFd`]
    #[deprecated(since = "0.4.1", note = "Use the provided AsRawFd implementation")]
    pub unsafe fn fd(&self) -> RawFd {
        ffi::libinput_get_fd(self.as_raw_mut())
    }

    /// Create a new instance of this type from a raw pointer.
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
    pub unsafe fn from_raw(ffi: *mut ffi::libinput) -> Self {
        Libinput {
            ffi: ffi::libinput_ref(ffi),
            _interface: None,
        }
    }
}

#[cfg(feature = "libinput_1_30")]
bitflags::bitflags! {
    /// Flags used for [`Libinput::plugin_system_load_plugins`]
    #[doc(alias = "libinput_plugin_system_flags")]
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub struct PluginSystemFlags: u32 {}
}

#[cfg(feature = "libinput_1_30")]
impl Libinput {
    /// Add the default plugin lookup paths
    #[doc(alias = "libinput_plugin_system_append_default_paths")]
    pub fn plugin_system_append_default_paths(&self) {
        unsafe { ffi::libinput_plugin_system_append_default_paths(self.as_raw_mut()) };
    }

    /// Appends the given directory path to the libinput plugin lookup path.
    /// If the path is already in the lookup paths, this function does nothing.
    #[doc(alias = "libinput_plugin_system_append_path")]
    pub fn plugin_system_append_path(&self, path: &str) {
        let path = CString::new(path).expect("Plugin path contained a null-byte");
        unsafe { ffi::libinput_plugin_system_append_path(self.as_raw_mut(), path.as_ptr()) };
    }

    /// Load the plugins from the set of lookup paths.
    /// This function does nothing if no plugin paths have been configured,
    /// see [`Self::plugin_system_append_default_paths`] and [`Self::plugin_system_append_path`]
    #[doc(alias = "libinput_plugin_system_load_plugins")]
    pub fn plugin_system_load_plugins(&self, flags: PluginSystemFlags) -> IoResult<()> {
        unsafe {
            match ffi::libinput_plugin_system_load_plugins(self.as_raw_mut(), flags.bits()) {
                0 => Ok(()),
                x if x < 0 => Err(IoError::from_raw_os_error(-x)),
                _ => unreachable!(),
            }
        }
    }
}

impl AsRawFd for Libinput {
    fn as_raw_fd(&self) -> RawFd {
        unsafe { ffi::libinput_get_fd(self.as_raw_mut()) }
    }
}

impl AsFd for Libinput {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.as_raw_fd()) }
    }
}
