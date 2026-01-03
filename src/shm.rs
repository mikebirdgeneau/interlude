use anyhow::Result;
use memmap2::MmapMut;
use rustix::fs::{memfd_create, MemfdFlags};
use std::os::fd::{AsFd, OwnedFd};

pub struct ShmBuffer {
    pub fd: OwnedFd,
    pub map: MmapMut,
    pub len: usize,
}

pub fn create_shm(len: usize) -> Result<ShmBuffer> {
    let fd = memfd_create("interlude-shm", MemfdFlags::CLOEXEC)?;
    rustix::fs::ftruncate(&fd, len as u64)?;

    let map = unsafe { MmapMut::map_mut(fd.as_fd())? };

    Ok(ShmBuffer { fd, map, len })
}
