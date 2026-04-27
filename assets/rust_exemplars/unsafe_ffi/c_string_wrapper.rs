// Sound FFI wrappers for C-string interop: borrow (length read), Rust→C
// ownership transfer (string build), and reclamation (string free). Each
// `extern "C" fn` body is panic-isolated via `catch_unwind` so a Rust
// panic returns a sentinel value rather than aborting the process
// (axiom 100). Ownership crossing the boundary is documented in each
// fn's rustdoc; the producer/reclaimer pair must be matched (axiom 102).
//
// Rust 2024 syntax: `unsafe extern "C"` on the fn declares "calling this
// requires the documented preconditions"; `#[unsafe(no_mangle)]` opts in
// to symbol mangling suppression (the `unsafe` keyword on the attribute
// reflects that name collisions across crates are a soundness hazard).

use std::ffi::{c_char, CStr, CString};
use std::panic::catch_unwind;

/// Returns the length of a NUL-terminated C string in bytes (excluding NUL).
/// Returns 0 if `s` is null or any internal panic occurs.
///
/// # Safety
/// `s` must be a valid pointer to a NUL-terminated C string for the duration
/// of the call, and no other mutator may modify the string while this runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ffi_string_len(s: *const c_char) -> usize {
    if s.is_null() {
        return 0;
    }
    catch_unwind(|| {
        // SAFETY: caller-documented non-null and NUL-terminated precondition;
        // `CStr::from_ptr` borrows for the duration of this call only.
        unsafe { CStr::from_ptr(s) }.to_bytes().len()
    })
    .unwrap_or(0)
}

/// Builds a Rust-owned C string and transfers ownership to the caller. The
/// caller MUST release the returned pointer with `ffi_string_free` and no
/// other deallocator. Returns null on internal failure (allocation failure,
/// embedded NUL, panic).
#[unsafe(no_mangle)]
pub extern "C" fn ffi_string_make() -> *mut c_char {
    catch_unwind(|| CString::new("hello from rust").map_or(std::ptr::null_mut(), CString::into_raw))
        .unwrap_or(std::ptr::null_mut())
}

/// Reclaims and frees a string previously returned by `ffi_string_make`.
///
/// # Safety
/// `s` must be a pointer obtained from `ffi_string_make` and not yet freed
/// by any other path. Passing a pointer from any other source is UB because
/// `CString::from_raw` requires the same allocator that produced it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ffi_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // CString::Drop is dealloc-only and unlikely to panic, but every
    // extern "C" fn body should be panic-isolated per axiom 100 — a panic
    // unwinding into C is UB under the default extern "C" ABI. The
    // exemplar should demonstrate the pattern consistently across all
    // three FFI functions, not just two.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: caller-documented provenance precondition: `s` was produced
        // by `CString::into_raw` in `ffi_string_make` and has not been freed.
        drop(unsafe { CString::from_raw(s) });
    }));
}
