//! Link stubs for incomplete `ext4-lwext4-sys` builds on MSVC.
//!
//! The published crate's Windows build of lwext4 omits xattr object files, so
//! the final link fails with LNK2019 for these six symbols. We do not call
//! xattr APIs; these stubs only satisfy the linker.

#![allow(non_snake_case, unused_variables, dead_code)]

use std::os::raw::{c_char, c_int, c_void};

#[no_mangle]
pub unsafe extern "C" fn ext4_extract_xattr_name(
    _full_name: *const c_char,
    _name_len: *mut usize,
    _prefix_len: *mut usize,
) -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn ext4_get_xattr_name_prefix(
    _prefix_index: usize,
    _len: *mut usize,
) -> *const c_char {
    std::ptr::null()
}

#[no_mangle]
pub unsafe extern "C" fn ext4_xattr_list(
    _inode_ref: *mut c_void,
    _list: *mut c_char,
    _size: usize,
    _ret_size: *mut usize,
) -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn ext4_xattr_get(
    _inode_ref: *mut c_void,
    _name: *const c_char,
    _buf: *mut c_void,
    _buf_size: usize,
    _data_size: *mut usize,
) -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn ext4_xattr_set(
    _inode_ref: *mut c_void,
    _name: *const c_char,
    _value: *const c_void,
    _value_size: usize,
) -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn ext4_xattr_remove(
    _inode_ref: *mut c_void,
    _name: *const c_char,
) -> c_int {
    -1
}
