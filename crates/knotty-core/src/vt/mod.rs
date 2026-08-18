//! The safe boundary over the VT engine.
//!
//! knotty owns this layer rather than taking a third-party one: the surface it
//! reaches is a fraction of a general-purpose wrapper's, and every raw pointer
//! it turns into a Rust value is one knotty has audited. cf.
//! `docs/adr/0012-own-the-binding-layer.md`
//!
//! It is not a renaming layer. The flattening loop that reads the engine's
//! render state into a [`Snapshot`] lives in [`capture`], which is what keeps
//! contracts the engine states in prose — a grapheme buffer sized by the
//! length the engine reports, a clipboard array that is null when empty —
//! enforceable in one place instead of at every call site. No engine type
//! appears in a signature outside this module. cf.
//! `docs/adr/0004-hide-vt-engine-types.md`
//!
//! The crate denies `unsafe` and lifts it here. This is the larger of the two
//! places it is lifted — the other is a single pre-exec hook in [`crate::io`],
//! which has no safe spelling. What the C API asks of a caller and what this
//! module does about it is written at each call.

// The one place the crate's ban is lifted. cf. `lib.rs`
#![allow(unsafe_code)]

mod capture;

use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr;

use libghostty_vt_sys as ffi;

use crate::listener::{ClipboardRefusal, Listener, Representation};
use crate::queue::ClipboardTarget;
use crate::session::SelectionRange;
use crate::{Error, Result};

/// DEC mode 2026, synchronized output.
///
/// Named by number because the engine takes a number: a DEC mode is its value,
/// and only ANSI modes carry a bit alongside it.
const SYNC_OUTPUT: ffi::Mode = 2026;

/// What knotty answers a device attributes query with.
///
/// A VT220 with color, which is what the engine implements. DA2's firmware
/// field is a version number and stays 0 while knotty is 0.x — it is set by
/// hand, not from the crate version. DA3's unit id is meaningless for an
/// emulator.
//
// The one piece of knotty's answering policy that lives in here rather than in
// `listener`: the engine's own struct is the only way to spell it.
const DEVICE_ATTRIBUTES: ffi::DeviceAttributes = ffi::DeviceAttributes {
    primary: ffi::DeviceAttributesPrimary {
        conformance_level: ffi::DA_CONFORMANCE_VT220,
        features: {
            let mut features = [0; 64];
            features[0] = ffi::DA_FEATURE_ANSI_COLOR;
            features
        },
        num_features: 1,
    },
    secondary: ffi::DeviceAttributesSecondary {
        device_type: ffi::DA_DEVICE_TYPE_VT220,
        firmware_version: 0,
        rom_cartridge: 0,
    },
    tertiary: ffi::DeviceAttributesTertiary { unit_id: 0 },
};

/// A terminal, its render state, and the iterators a capture walks it with.
///
/// All four handles are the engine's, and none of them is thread-safe: the
/// thread that built a terminal is the only one that may drive it. cf.
/// `docs/adr/0003-snapshot-mailbox.md`
pub struct Terminal {
    raw: ffi::Terminal,
    render: ffi::RenderState,
    rows: ffi::RenderStateRowIterator,
    cells: ffi::RenderStateRowCells,
    /// Owned outright rather than held in a `Box`, so that the pointer the
    /// engine keeps stays valid while the terminal around it is borrowed.
    listener: *mut Listener,
}

impl Terminal {
    /// Build a terminal of `cols` by `rows` with `max_scrollback` lines behind
    /// it, and wire `listener` to it.
    pub fn new(cols: u16, rows: u16, max_scrollback: usize, listener: Listener) -> Result<Self> {
        let options = ffi::TerminalOptions {
            cols,
            rows,
            max_scrollback,
        };
        let mut raw: ffi::Terminal = ptr::null_mut();
        // SAFETY: a null allocator asks for the engine's own, and `raw` is a
        // handle-sized out parameter.
        check(unsafe { ffi::ghostty_terminal_new(ptr::null(), &raw mut raw, options) })?;

        // Built one field at a time so that a failure part-way through is
        // released by this value's own `Drop` rather than by a second copy of
        // it written out here.
        let mut terminal = Self {
            raw,
            render: ptr::null_mut(),
            rows: ptr::null_mut(),
            cells: ptr::null_mut(),
            listener: Box::into_raw(Box::new(listener)),
        };
        // SAFETY: as above, for each of the three handles a capture needs.
        check(unsafe { ffi::ghostty_render_state_new(ptr::null(), &raw mut terminal.render) })?;
        check(unsafe {
            ffi::ghostty_render_state_row_iterator_new(ptr::null(), &raw mut terminal.rows)
        })?;
        check(unsafe {
            ffi::ghostty_render_state_row_cells_new(ptr::null(), &raw mut terminal.cells)
        })?;

        terminal.listen()?;
        Ok(terminal)
    }

    /// Give the engine the callbacks and the userdata they are handed back
    /// with.
    fn listen(&self) -> Result<()> {
        // The engine takes a callback's address rather than a pointer to one.
        // Each goes past a function pointer type on the way, because a
        // function *item* is a zero-sized value whose address says nothing.
        self.set(ffi::TerminalOption::USERDATA, self.listener.cast())?;
        self.set(
            ffi::TerminalOption::WRITE_PTY,
            on_pty_write as PtyWriteFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::BELL,
            on_bell as BellFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::ENQUIRY,
            on_enquiry as AnswerFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::XTVERSION,
            on_xtversion as AnswerFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::DEVICE_ATTRIBUTES,
            on_device_attributes as DeviceAttributesFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::CLIPBOARD_WRITE,
            on_clipboard_write as ClipboardWriteFn as *const c_void,
        )?;

        // A clipboard read gets no callback because the engine drops the
        // request without telling us: there is nothing to answer, so nothing
        // goes out. The pixel size and color scheme queries get none either —
        // both are answers the core cannot know. cf.
        // `docs/adr/0007-input-security.md`
        Ok(())
    }

    /// Process `bytes` to completion.
    ///
    /// The engine's callbacks all fire inside this call, on this thread. It
    /// cannot fail: malformed input is the case it exists for.
    pub fn feed(&mut self, bytes: &[u8]) {
        // SAFETY: the engine reads `len` bytes and keeps none of them.
        unsafe { ffi::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) }
    }

    /// Whether a synchronized output block is open as of now.
    pub fn sync_output_open(&self) -> Result<bool> {
        let mut open = false;
        // SAFETY: `open` is the `bool` out parameter the call documents.
        check(unsafe { ffi::ghostty_terminal_mode_get(self.raw, SYNC_OUTPUT, &raw mut open) })?;
        Ok(open)
    }

    /// Whether the active screen has a selection.
    ///
    /// Only that there is one: the snapshot carries where it falls per row,
    /// and the engine's answer is borrowed grid references that go stale on
    /// the next feed.
    pub fn has_selection(&self) -> Result<bool> {
        // Not `get`: its reader zeroes the value, which leaves the sized
        // struct's own size unfilled, and `check` would turn the answer this
        // asks for into an error.
        let mut selection = ffi::sized!(ffi::Selection);
        // SAFETY: the tag's documented output type, sized as its ABI asks.
        let code = unsafe {
            ffi::ghostty_terminal_get(
                self.raw,
                ffi::TerminalData::SELECTION,
                (&raw mut selection).cast(),
            )
        };
        match code {
            ffi::Result::SUCCESS => Ok(true),
            ffi::Result::NO_VALUE => Ok(false),
            _ => Err(Error::Engine),
        }
    }

    /// Select a range of the viewport, or clear the selection with `None`.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] when either endpoint is outside the viewport.
    pub fn set_selection(&mut self, range: Option<SelectionRange>) -> Result<()> {
        let Some(range) = range else {
            return self.set(ffi::TerminalOption::SELECTION, ptr::null());
        };

        let selection = ffi::Selection {
            start: self.grid_ref(range.start_x, range.start_y)?,
            end: self.grid_ref(range.end_x, range.end_y)?,
            rectangle: range.rectangle,
            ..ffi::sized!(ffi::Selection)
        };
        // The engine copies the selection and converts it to tracked state
        // during the call, so neither this value nor the references in it have
        // to outlive the call.
        self.set(
            ffi::TerminalOption::SELECTION,
            ptr::from_ref(&selection).cast(),
        )
    }

    /// Resolve a viewport coordinate to a reference the engine can hold on to.
    ///
    /// The result is good until the next thing that moves the grid, which is
    /// why it never leaves the call that asked for it.
    fn grid_ref(&self, x: u16, y: u16) -> Result<ffi::GridRef> {
        let point = ffi::Point {
            tag: ffi::PointTag::VIEWPORT,
            value: ffi::PointValue {
                coordinate: ffi::PointCoordinate { x, y: u32::from(y) },
            },
        };
        let mut resolved = ffi::sized!(ffi::GridRef);
        // SAFETY: `resolved` is the sized out parameter the call documents,
        // with its size filled in.
        let result = unsafe { ffi::ghostty_terminal_grid_ref(self.raw, point, &raw mut resolved) };
        if result == ffi::Result::SUCCESS {
            return Ok(resolved);
        }
        // The one way this fails that a caller can do anything about, and the
        // only one an app can cause: a point outside the coordinate space.
        Err(Error::OutOfRange)
    }

    /// Read a borrowed string out of the terminal.
    ///
    /// The engine lends it until the next feed, which is what the borrow of
    /// `self` stands for. Nothing promises it is UTF-8 in the type, so it is
    /// checked here rather than assumed — the whole reason this layer exists.
    fn text(&self, data: ffi::TerminalData::Type) -> Result<&str> {
        // SAFETY: the tag's documented output type.
        let string: ffi::String = unsafe { self.get(data) }?;
        if string.ptr.is_null() {
            // The C header declares the pointer non-optional and the engine
            // returns a static sentinel for the empty string. Checked anyway:
            // a null slice is undefined behaviour even at length zero, and
            // this is the boundary that is supposed to catch that.
            return Ok("");
        }
        // SAFETY: non-null, and lent for as long as the terminal is not fed.
        let bytes = unsafe { std::slice::from_raw_parts(string.ptr, string.len) };
        std::str::from_utf8(bytes).map_err(|_| Error::Engine)
    }

    /// Read a terminal value into `T`.
    ///
    /// # Safety
    ///
    /// `T` must be the output type `data` documents.
    unsafe fn get<T>(&self, data: ffi::TerminalData::Type) -> Result<T> {
        // SAFETY: the caller's, as declared.
        unsafe { read(|out| ffi::ghostty_terminal_get(self.raw, data, out)) }
    }

    /// Set a terminal option to whatever `value` points at, or to nothing.
    fn set(&self, option: ffi::TerminalOption::Type, value: *const c_void) -> Result<()> {
        // SAFETY: each caller passes the input type its option documents, and
        // the engine reads it during the call.
        check(unsafe { ffi::ghostty_terminal_set(self.raw, option, value) })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: each handle was made by its matching constructor and is
        // freed once. A part-built terminal leaves the rest null, which is why
        // each is checked — only some of these calls document taking null.
        unsafe {
            if !self.cells.is_null() {
                ffi::ghostty_render_state_row_cells_free(self.cells);
            }
            if !self.rows.is_null() {
                ffi::ghostty_render_state_row_iterator_free(self.rows);
            }
            if !self.render.is_null() {
                ffi::ghostty_render_state_free(self.render);
            }
            // Before the listener, which the engine may still call from here.
            ffi::ghostty_terminal_free(self.raw);
            drop(Box::from_raw(self.listener));
        }
    }
}

/// Turn a checked result code into knotty's.
///
/// Every failure the engine reports is the same news up here: the engine
/// refused, and the caller's own error tells what it was doing at the time.
fn check(code: ffi::Result::Type) -> Result<()> {
    if code == ffi::Result::SUCCESS {
        return Ok(());
    }
    Err(Error::Engine)
}

/// Hand `get` somewhere to put a `T` and take back what it put there.
///
/// The engine's readers are all this shape — a tag, a pointer to write
/// through, and a result code — and the delicate half is the same every time.
/// It lives here once rather than at each of the five tag namespaces, which
/// keep their own entry points so that a tag cannot be used against the handle
/// it does not belong to.
///
/// # Safety
///
/// `T` must be the output type the tag `get` passes documents.
unsafe fn read<T>(get: impl FnOnce(*mut c_void) -> ffi::Result::Type) -> Result<T> {
    let mut value = MaybeUninit::<T>::zeroed();
    check(get(value.as_mut_ptr().cast()))?;
    // SAFETY: a successful call initializes the value, and the caller promised
    // it is a `T`.
    Ok(unsafe { value.assume_init() })
}

/// The engine's callback types with the null it also allows taken off, so that
/// the functions below can be coerced to pointers.
type PtyWriteFn = unsafe extern "C" fn(ffi::Terminal, *mut c_void, *const u8, usize);
type BellFn = unsafe extern "C" fn(ffi::Terminal, *mut c_void);
type AnswerFn = unsafe extern "C" fn(ffi::Terminal, *mut c_void) -> ffi::String;
type DeviceAttributesFn =
    unsafe extern "C" fn(ffi::Terminal, *mut c_void, *mut ffi::DeviceAttributes) -> bool;
type ClipboardWriteFn = unsafe extern "C" fn(
    ffi::Terminal,
    *mut c_void,
    *const ffi::ClipboardWrite,
) -> ffi::ClipboardWriteResult::Type;

/// Borrow an engine string as bytes, whatever it holds.
///
/// # Safety
///
/// The bytes must be ones the engine lends for at least `'a`.
unsafe fn bytes_of<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() {
        // A null array is not a slice at any length, empty included.
        return &[];
    }
    // SAFETY: non-null, and lent for `'a` by the caller's promise.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// Borrow the listener the engine was given.
///
/// # Safety
///
/// `userdata` must be the pointer a live [`Terminal`] registered, and no other
/// borrow of it may be live. The engine calls back only from inside a feed, so
/// the terminal that owns it is alive and nothing else is looking at it.
unsafe fn listener_of<'a>(userdata: *mut c_void) -> &'a mut Listener {
    // SAFETY: the caller's, as declared.
    unsafe { &mut *userdata.cast::<Listener>() }
}

unsafe extern "C" fn on_pty_write(
    _terminal: ffi::Terminal,
    userdata: *mut c_void,
    ptr: *const u8,
    len: usize,
) {
    // SAFETY: called from inside a feed on the terminal that registered it.
    let listener = unsafe { listener_of(userdata) };
    // SAFETY: the engine lends `len` bytes for the length of this call.
    let bytes = unsafe { bytes_of(ptr, len) };
    (listener.pty_write)(bytes);
}

unsafe extern "C" fn on_bell(_terminal: ffi::Terminal, userdata: *mut c_void) {
    // SAFETY: as `on_pty_write`.
    let listener = unsafe { listener_of(userdata) };
    (listener.bell)();
}

unsafe extern "C" fn on_enquiry(_terminal: ffi::Terminal, userdata: *mut c_void) -> ffi::String {
    // SAFETY: as `on_pty_write`. The answer is a `'static` string of knotty's,
    // so the engine may read it whenever it likes.
    let listener = unsafe { listener_of(userdata) };
    ffi::String::from(listener.answerback)
}

unsafe extern "C" fn on_xtversion(_terminal: ffi::Terminal, userdata: *mut c_void) -> ffi::String {
    // SAFETY: as `on_enquiry`.
    let listener = unsafe { listener_of(userdata) };
    ffi::String::from(listener.version)
}

unsafe extern "C" fn on_device_attributes(
    _terminal: ffi::Terminal,
    _userdata: *mut c_void,
    out: *mut ffi::DeviceAttributes,
) -> bool {
    if out.is_null() {
        return false;
    }
    // SAFETY: non-null storage for one of these, handed over to be filled in.
    unsafe { *out = DEVICE_ATTRIBUTES };
    true
}

unsafe extern "C" fn on_clipboard_write(
    _terminal: ffi::Terminal,
    userdata: *mut c_void,
    write: *const ffi::ClipboardWrite,
) -> ffi::ClipboardWriteResult::Type {
    // SAFETY: as `on_pty_write`.
    let listener = unsafe { listener_of(userdata) };
    // SAFETY: the engine lends one of these for the length of this call.
    // `as_ref` is the checked spelling: a null write is refused below rather
    // than dereferenced.
    let Some(write) = (unsafe { write.as_ref() }) else {
        return ffi::ClipboardWriteResult::UNSUPPORTED;
    };

    // A write carrying no representations is the C API asking for the
    // clipboard to be cleared, and it says so with a null array rather than an
    // empty one — which is not a slice, at any length. This check is one of
    // the two the facade was built for. cf. ADR 0012
    let contents = if write.contents.is_null() {
        &[][..]
    } else {
        // SAFETY: the engine lends the array for the length of this call.
        unsafe { std::slice::from_raw_parts(write.contents, write.contents_len) }
    };
    let target = match write.location {
        ffi::ClipboardLocation::SELECTION => ClipboardTarget::Selection,
        ffi::ClipboardLocation::PRIMARY => ClipboardTarget::Primary,
        // Standard, and anything a later engine adds: the system clipboard is
        // the one every protocol means by default.
        _ => ClipboardTarget::Standard,
    };

    // Handed on as knotty's own values, so that the table deciding what
    // becomes of them never sees an engine type — nor needs a terminal to be
    // called. cf. `03-core.md` C3
    let offered: Vec<Representation<'_>> = contents
        .iter()
        // SAFETY: both strings are borrowed from the engine for the length of
        // this call, which outlives the borrow below.
        .map(|content| unsafe {
            Representation {
                mime: bytes_of(content.mime.ptr, content.mime.len),
                data: bytes_of(content.data.ptr, content.data.len),
            }
        })
        .collect();

    match (listener.clipboard_write)(target, &offered) {
        Ok(()) => ffi::ClipboardWriteResult::SUCCESS,
        Err(ClipboardRefusal::Denied) => ffi::ClipboardWriteResult::DENIED,
        Err(ClipboardRefusal::InvalidData) => ffi::ClipboardWriteResult::INVALID_DATA,
        Err(ClipboardRefusal::Unsupported) => ffi::ClipboardWriteResult::UNSUPPORTED,
    }
}
