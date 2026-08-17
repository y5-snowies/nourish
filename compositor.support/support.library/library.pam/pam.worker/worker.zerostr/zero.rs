/// Overwrite every byte with zero using volatile writes so the
/// compiler cannot optimize them away as dead stores.
#[inline(never)]
pub(crate) fn zero_bytes(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        unsafe { std::ptr::write_volatile(b as *mut u8, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

use std::fmt;
use std::ops::Deref;

/// A `String` that overwrites its bytes with zeros on drop.
pub struct ZeroString(pub(crate) String);

impl ZeroString {
    /// Empty `ZeroString` preallocated to 256 bytes.
    pub fn new() -> Self {
        Self(String::with_capacity(256))
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(String::with_capacity(cap.max(256)))
    }

    pub fn push(&mut self, c: char) {
        self.0.push(c);
    }

    pub fn pop(&mut self) -> Option<char> {
        self.0.pop()
    }

    pub fn clear(&mut self) {
        let bytes = unsafe { self.0.as_bytes_mut() };
        zero_bytes(bytes);
        self.0.clear();
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn char_count(&self) -> usize {
        self.0.chars().count()
    }
}

impl Default for ZeroString {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for ZeroString {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ZeroString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ZeroString").field(&"[REDACTED]").finish()
    }
}

impl Clone for ZeroString {
    fn clone(&self) -> Self {
        let mut new = Self::with_capacity(self.0.len());
        new.0.push_str(&self.0);
        new
    }
}

impl Drop for ZeroString {
    fn drop(&mut self) {
        let bytes = unsafe { self.0.as_bytes_mut() };
        zero_bytes(bytes);
    }
}
