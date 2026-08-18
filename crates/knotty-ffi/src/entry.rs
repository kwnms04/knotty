//! Turning what a caller handed across the boundary into Rust values.
//!
//! Every entry point opens the same way, because the header promises the same
//! things of every one: a handle is not null, a run of bytes may be null only
//! when it is empty, an out parameter is not null and is left holding an empty
//! answer before anything that could fail. Written once here rather than at
//! each entry point — there are eleven, and M3's input path adds six more. cf.
//! `02-ffi.md`

use crate::KtStatus;

/// How an entry point came out.
///
/// `Err` is a refusal at the boundary: the call never reached the core, and
/// nothing of what it asked for happened. `Ok` is what the core answered,
/// which is not necessarily success — a full writer queue comes back that way.
pub type Answer = Result<KtStatus, KtStatus>;

/// Report an entry point's answer, whichever of the two it is.
pub fn answer(call: impl FnOnce() -> Answer) -> KtStatus {
    call().unwrap_or_else(|refusal| refusal)
}

/// Borrow the run a caller lent.
///
/// A run of no length is the one that may be null: there is nothing to read,
/// and a null slice is undefined behaviour even at length zero.
///
/// # Safety
///
/// `bytes` must point at `len` readable bytes, or be null when `len` is 0.
pub unsafe fn borrowed<'a>(bytes: *const u8, len: usize) -> Result<&'a [u8], KtStatus> {
    if len == 0 {
        return Ok(&[]);
    }
    if bytes.is_null() {
        return Err(KtStatus::NullArgument);
    }
    // SAFETY: non-null, and the caller's promise of `len` readable bytes.
    Ok(unsafe { std::slice::from_raw_parts(bytes, len) })
}

/// Borrow what a pointer names, refusing a null one.
///
/// # Safety
///
/// `ptr` must be null or point at a live `T` for the whole of the borrow.
pub unsafe fn at<'a, T>(ptr: *const T) -> Result<&'a T, KtStatus> {
    // SAFETY: the caller's, as declared. `as_ref` is the checked spelling.
    unsafe { ptr.as_ref() }.ok_or(KtStatus::NullArgument)
}

/// The same, for what the call is going to change.
///
/// # Safety
///
/// As [`at`], and nothing else may be looking at the `T` for the whole of the
/// borrow.
pub unsafe fn at_mut<'a, T>(ptr: *mut T) -> Result<&'a mut T, KtStatus> {
    // SAFETY: the caller's, as declared.
    unsafe { ptr.as_mut() }.ok_or(KtStatus::NullArgument)
}

/// Borrow an out parameter, first filling it in with what a call that got
/// nowhere leaves behind.
///
/// Written before anything else can fail, so that a caller reading `out` after
/// a refusal finds an empty answer rather than whatever was in its own
/// variable. A call that goes through overwrites it.
///
/// # Safety
///
/// As [`at_mut`]. `T` must also be a type that needs no dropping: assigning
/// over what is there would otherwise drop a value the caller never put
/// there.
pub unsafe fn out<'a, T>(ptr: *mut T, empty: T) -> Result<&'a mut T, KtStatus> {
    // SAFETY: the caller's, as declared.
    let place = unsafe { at_mut(ptr) }?;
    *place = empty;
    Ok(place)
}
