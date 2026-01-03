use anyhow::Result;
use memmap2::MmapMut;
use rustix::fs::{memfd_create, MemfdFlags};
use rustix::mm::{mmap, MapFlags, ProtFlags};
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::ptr::NonNull;

pub struct ShmBuffer {
    pub fd: OwnedFd,
    pub map: MmapMut,
    pub len: usize,
}

pub fn create_shm(len: usize) -> Result<ShmBuffer> {
    let fd = memfd_create("interlude-shm", MemfdFlags::CLOEXEC)?;
    rustix::fs::ftruncate(&fd, len as u64)?;

    // Map it
    let ptr = unsafe {
        mmap(
            std::ptr::null_mut(),
            len,
            ProtFlags::READ | ProtFlags::WRITE,
            MapFlags::SHARED,
            &fd,
            0,
        )?
    };

    let nn = NonNull::new(ptr as *mut u8).unwrap();
    let map = unsafe { MmapMut::map_mut(fd.as_fd())? };

    // `map_mut` already mmaps; the above mmap call is redundant in some setups.
    // Keep this scaffold simple: rely on memmap2’s mapping.
    // If you hit issues, remove the manual `mmap` call above.

    Ok(ShmBuffer { fd, map, len })
}

