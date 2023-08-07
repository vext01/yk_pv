//! This implements support for the GDB JIT Compilation Interface described here:
//! https://sourceware.org/gdb/onlinedocs/gdb/JIT-Interface.html
//!
//! This allows gdb to recognise our hot-loaded code, so that we can have debugging info
//! interleaved with the disassembly view.

use byteorder::{NativeEndian, WriteBytesExt};
use libc;
use std::{
    collections::HashMap,
    ffi::{c_char, c_int, c_void},
    io::Write,
    mem,
    path::Path,
    ptr,
    sync::Mutex,
};

#[repr(u32)]
pub enum JitActionsT {
    JitNoAction = 0u32,
    JitRegisterFn = 1u32,
    JitUnregisterFn = 2u32,
}

#[repr(C)]
pub struct JitCodeEntry {
    next_entry: *mut Self,
    prev_entry: *mut Self,
    symfile_addr: *const c_char,
    symfile_size: u64,
}

#[repr(C)]
pub struct JitDescriptor {
    version: u32,
    action_flag: u32,
    relevant_entry: *mut JitCodeEntry,
    first_entry: *mut JitCodeEntry,
}

// We know what we are doing (tm).
unsafe impl Send for JitDescriptor {}
unsafe impl Sync for JitDescriptor {}

/// GDB regognises calls to this function to detect when code is being loaded.
#[inline(never)]
#[no_mangle]
pub extern "C" fn __jit_debug_register_code() { }

/// Guards `__jit_debug_descriptor`. Ideally that would live inside this mutex, but then gdb
/// wouldn't understand it.
///
/// FIXME: use Jake's design: https://play.rust-lang.org/?version=stable&mode=debug&edition=2021&gist=56f43e6636ccf2b6a018418956747cba
static LOCK: Mutex<()> = Mutex::new(());

// GDB also recognises this symbol specially.
#[allow(non_upper_case_globals)]
#[no_mangle]
pub static mut __jit_debug_descriptor: JitDescriptor = JitDescriptor {
    version: 1,
    action_flag: 0,
    relevant_entry: ptr::null_mut(),
    first_entry: ptr::null_mut(),
};

#[cfg(target_pointer_width = "64")]
fn write_usize(into: &mut dyn Write, data: usize) {
    into.write_u64::<NativeEndian>(data as u64).unwrap(); // cast safe on x86_64.
}

fn write_c_int(into: &mut dyn Write, data: c_int) {
    into.write_int::<NativeEndian>(data as i64, mem::size_of::<c_int>()).unwrap(); // cast safe on x86_64.
}

pub(crate) fn register_jitted_code(code: *const c_void, code_size: usize, lineinfo: &HashMap<usize, usize>, src_filename: &Path) {
    // FIXME: check all allocations.

    // Ensure we have exclusive access to mutate __jit_debug_descriptor.
    let lock = LOCK.lock();

    let desc = unsafe { (&mut __jit_debug_descriptor) as *mut JitDescriptor };
    let old_first = unsafe { (*desc).first_entry };

    // Write the debug info into a buffer in our custom format:
    //
    // code_vaddr: usize
    // code_size: usize
    // num_lineinfo_pairs: usize
    // lineinfo_key[0]: c_int
    // padding[0]: c_int
    // lineinfo_val[0]: usize
    // ...
    // lineinfo_key[num_lineinfo_pairs - 1]: c_int
    // padding[num_lineinfo_pairs]: c_int
    // lineinfo_val[num_lineinfo_pairs - 1]: usize
    // src_filename: char[src_filename_len] (null terminated)
    //
    // Note that the lineinfo pairs are designed to be ABI compatible with gdb's `struct
    // gdb_line_mapping`. FIXME: not sure the padding is right for all arches..
    //
    // Note that `src_filename` is at the end to ensure that all over fields are aligned.
    let mut payload: Vec<u8> = Vec::new();
    // code_vaddr
    write_usize(&mut payload, code as usize); // cast safe by definition.
    // code_size
    write_usize(&mut payload, code_size);
    // num_lineinfo_pairs
    write_usize(&mut payload, lineinfo.len());
    // lineinfo_pairs
    // FIXME consider using an ordered data structure? BTreeMap?
    let mut pairs: Vec<_> = lineinfo.into_iter().collect();
    pairs.sort_by(|x,y| x.0.cmp(&y.0));
    for (k, v) in pairs {
        // FIXME: reverse k + v in map?
        write_c_int(&mut payload, c_int::try_from(*v).unwrap());
        write_c_int(&mut payload, 0); // padding
        write_usize(&mut payload, *k);
    }
    // src_filename
    payload.extend(src_filename.to_str().unwrap().as_bytes());

    // Make a new linked list node.
    let new = JitCodeEntry {
        next_entry: old_first,
        prev_entry: ptr::null_mut(),
        symfile_addr: payload.as_ptr() as *const i8,
        symfile_size: u64::try_from(payload.len()).unwrap(),
    };

    // Stick it on the heap.
    // FIXME: Do we even need it on the heap?
    let new_raw = unsafe { libc::malloc(mem::size_of::<JitCodeEntry>()) } as *mut JitCodeEntry;
    unsafe { ptr::write(new_raw, new) };

    // Patch up remaining linked list links.
    if !old_first.is_null() {
        unsafe { ptr::write(ptr::addr_of_mut!((*old_first).prev_entry), new_raw) };
    }
    unsafe { ptr::write(ptr::addr_of_mut!((*desc).first_entry), new_raw) };

    // Inform gdb of the new code.
    unsafe { ptr::write(ptr::addr_of_mut!((*desc).relevant_entry), new_raw) };
    unsafe {
        ptr::write(
            ptr::addr_of_mut!((*desc).action_flag),
            JitActionsT::JitRegisterFn as u32,
        )
    };
    __jit_debug_register_code();

    drop(lock); // hold lock until here: there's no dependency between the lock and what it
                // guards, so without this Rust may drop the lock too soon.
}

fn print_list() {
    let mut entry = unsafe { __jit_debug_descriptor.first_entry };
    // Find the last entry in the linked list. This will beocme the `prev_entry` in the new entry.
    while !entry.is_null() {
        unsafe {
            dbg!((*entry).next_entry);
            dbg!((*entry).prev_entry);
            dbg!((*entry).symfile_addr);
            dbg!((*entry).symfile_size);
        };
        entry = unsafe { (*entry).next_entry };
    }
}
