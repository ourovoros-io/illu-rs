// Incremental initialization of a non-Default struct using MaybeUninit and
// `&raw mut`. The pattern is needed when the value is filled in stages
// and you cannot construct it via struct-literal syntax — common at FFI
// boundaries where C fills in a buffer field-by-field.
//
// The critical invariant: never materialize a `&mut Frame` pointing at the
// partially-initialized value. The `&mut` is invalid the moment it exists,
// regardless of whether anything reads through it. `&raw mut (*ptr).field`
// produces a place pointer to one field without going through `&mut`.
//
// Each unsafe block is the smallest possible scope (axiom 96) with a
// SAFETY comment naming the invariants it relies on (axiom 94).

use std::mem::MaybeUninit;

pub struct Frame {
    pub seq: u32,
    pub payload: [u8; 64],
    pub timestamp_ns: u64,
}

/// Build a Frame by initializing each field through a raw pointer.
///
/// `source` is copied into `payload` (truncated to 64 bytes if longer; the
/// remainder of `payload` is zeroed).
pub fn build_frame(seq: u32, source: &[u8], timestamp_ns: u64) -> Frame {
    let mut uninit = MaybeUninit::<Frame>::uninit();
    let ptr = uninit.as_mut_ptr();

    // SAFETY: `&raw mut` yields a place pointer to `seq` without
    // materializing a `&mut Frame` over the partially-initialized value
    // (which would be UB). `.write` initializes the field through the
    // raw pointer, depositing `seq` into the `Frame`'s `seq` slot.
    unsafe {
        (&raw mut (*ptr).seq).write(seq);
    }

    // SAFETY: `&raw mut (*ptr).payload` is the place pointer to the
    // `[u8; 64]` payload field; `cast()` reinterprets it as `*mut u8` so we
    // can index byte-by-byte. The cast preserves provenance and validity.
    let payload_ptr: *mut u8 = unsafe { (&raw mut (*ptr).payload).cast() };
    let copy_len = source.len().min(64);
    for (i, byte) in source.iter().copied().take(copy_len).enumerate() {
        // SAFETY: `payload` is `[u8; 64]` and `payload_ptr` points at its
        // first element. `i < copy_len <= 64`, so `payload_ptr.add(i)` is
        // within the field's allocation.
        unsafe {
            payload_ptr.add(i).write(byte);
        }
    }
    for i in copy_len..64 {
        // SAFETY: same bounds reasoning as above; `i < 64` so
        // `payload_ptr.add(i)` is within the 64-byte field.
        unsafe {
            payload_ptr.add(i).write(0);
        }
    }

    // SAFETY: see the `seq` block above for the field-pointer rationale;
    // the same applies to `timestamp_ns`.
    unsafe {
        (&raw mut (*ptr).timestamp_ns).write(timestamp_ns);
    }

    // SAFETY: every field of `Frame` has now been initialized — `seq`,
    // all 64 bytes of `payload`, and `timestamp_ns`. `assume_init` requires
    // the `MaybeUninit` to be in a valid state for `Frame`; that holds.
    unsafe { uninit.assume_init() }
}
