use anyhow::{anyhow, Result};
use crossbeam_channel::Sender;
use std::collections::HashMap;

use wayland_client::{
    protocol::{
        wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_keyboard, wl_output::WlOutput,
        wl_registry, wl_seat::WlSeat, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ext_session_lock_v1::ExtSessionLockV1,
};

use xkbcommon::xkb;

use crate::tiny_font::draw_text_rgba;

#[derive(Debug, Clone, Copy)]
pub enum UiEvent {
    PressZ,
    PressEnter,
}

#[derive(Debug, Clone)]
pub enum UiMode {
    BreakDue { snooze_secs: u64, can_snooze: bool },
    OnBreak { secs_left: u64 },
    BreakFinished,
}

pub struct Locker {
    conn: Connection,
    event_queue: EventQueue<State>,
    state: State,
}

struct SurfaceCtx {
    output: WlOutput,
    wl_surface: WlSurface,
    lock_surface: ExtSessionLockSurfaceV1,
    width: u32,
    height: u32,

    // SHM objects (recreated on resize/configure)
    shm_pool: Option<WlShmPool>,
    buffer: Option<WlBuffer>,
    shm_bytes: Vec<u8>,
    stride: i32,
}

struct State {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    seat: Option<WlSeat>,
    session_lock_mgr: Option<ExtSessionLockManagerV1>,

    outputs: Vec<WlOutput>,
    surfaces: Vec<SurfaceCtx>,

    session_lock: Option<ExtSessionLockV1>,

    keyboard: Option<wl_keyboard::WlKeyboard>,
    xkb_context: xkb::Context,
    xkb_keymap: Option<xkb::Keymap>,
    xkb_state: Option<xkb::State>,

    ui_mode: UiMode,
    tx_ui: Sender<UiEvent>,

    // track wl_output -> last known size, until configure gives us real values
    output_sizes: HashMap<u32, (u32, u32)>,
}

impl Locker {
    pub fn new(tx_ui: Sender<UiEvent>) -> Result<Self> {
        let conn = Connection::connect_to_env()?;
        let (globals, event_queue) = wayland_client::globals::registry_queue_init(&conn)?;
        let qh = event_queue.handle();

        let mut state = State {
            compositor: None,
            shm: None,
            seat: None,
            session_lock_mgr: None,
            outputs: vec![],
            surfaces: vec![],
            session_lock: None,
            keyboard: None,
            xkb_context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            xkb_keymap: None,
            xkb_state: None,
            ui_mode: UiMode::BreakDue {
                snooze_secs: 300,
                can_snooze: true,
            },
            tx_ui,
            output_sizes: HashMap::new(),
        };

        for g in globals.list() {
            match g.interface.as_str() {
                "wl_compositor" => state.compositor = Some(globals.bind(&qh, g.name, g.version, ())?),
                "wl_shm" => state.shm = Some(globals.bind(&qh, g.name, g.version, ())?),
                "wl_seat" => state.seat = Some(globals.bind(&qh, g.name, g.version, ())?),
                "wl_output" => {
                    let out: WlOutput = globals.bind(&qh, g.name, g.version, ())?;
                    state.outputs.push(out);
                }
                "ext_session_lock_manager_v1" => {
                    state.session_lock_mgr = Some(globals.bind(&qh, g.name, g.version, ())?);
                }
                _ => {}
            }
        }

        if state.compositor.is_none()
            || state.shm.is_none()
            || state.seat.is_none()
            || state.session_lock_mgr.is_none()
        {
            return Err(anyhow!(
                "Missing required Wayland globals (compositor/shm/seat/session_lock_manager)"
            ));
        }

        // Create keyboard
        let seat = state.seat.clone().unwrap();
        state.keyboard = Some(seat.get_keyboard(&qh, ()));

        let mut locker = Self {
            conn,
            event_queue,
            state,
        };

        // Let initial globals events settle
        locker.roundtrip()?;
        Ok(locker)
    }

    pub fn roundtrip(&mut self) -> Result<()> {
        self.event_queue.roundtrip(&mut self.state)?;
        Ok(())
    }

    pub fn pump(&mut self) -> Result<()> {
        self.event_queue.dispatch_pending(&mut self.state)?;
        self.conn.flush()?;
        Ok(())
    }

    pub fn set_mode(&mut self, mode: UiMode) {
        self.state.ui_mode = mode;
        self.redraw_all();
    }

    pub fn is_locked(&self) -> bool {
        self.state.session_lock.is_some()
    }

    pub fn lock(&mut self) -> Result<()> {
        if self.is_locked() {
            return Ok(());
        }
        let mgr = self.state.session_lock_mgr.clone().unwrap();
        let qh = self.event_queue.handle();

        let lock = mgr.lock(&qh, ());
        self.state.session_lock = Some(lock);

        // create a surface per output
        self.state.surfaces.clear();
        let compositor = self.state.compositor.clone().unwrap();

        for out in self.state.outputs.iter().cloned() {
            let wl_surface = compositor.create_surface(&qh, ());
            let lock_surface = self
                .state
                .session_lock
                .as_ref()
                .unwrap()
                .get_lock_surface(&wl_surface, &out, &qh, ());

            // placeholder until configure
            let (w, h) = (1920u32, 1080u32);

            self.state.surfaces.push(SurfaceCtx {
                output: out,
                wl_surface,
                lock_surface,
                width: w,
                height: h,
                shm_pool: None,
                buffer: None,
                shm_bytes: vec![],
                stride: (w as i32) * 4,
            });
        }

        // roundtrip so we receive configure sizes
        self.roundtrip()?;

        self.redraw_all();
        Ok(())
    }

    pub fn unlock(&mut self) {
        if let Some(lock) = self.state.session_lock.take() {
            lock.unlock();
        }
        self.state.surfaces.clear();
    }

    fn redraw_all(&mut self) {
        for i in 0..self.state.surfaces.len() {
            self.redraw_surface(i);
        }
    }

    fn redraw_surface(&mut self, idx: usize) {
        let qh = self.event_queue.handle();
        let shm = match self.state.shm.clone() {
            Some(s) => s,
            None => return,
        };

        let (w, h) = {
            let s = &self.state.surfaces[idx];
            (s.width, s.height)
        };

        let stride = (w as i32) * 4;
        let size = (stride as usize) * (h as usize);

        // Allocate a simple RGBA buffer
        let mut bytes = vec![0u8; size];

        // Dim background: mostly opaque black
        for px in bytes.chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 0, 0, 220]);
        }

        let white = [235, 235, 235, 255];
        let (l1, l2, l3) = match &self.state.ui_mode {
            UiMode::BreakDue { snooze_secs, can_snooze } => {
                let l1 = "INTERLUDE";
                let l2 = "ENTER: BEGIN";
                let l3 = if *can_snooze {
                    let m = snooze_secs / 60;
                    let s = snooze_secs % 60;
                    format!("Z: SNOOZE {}:{:02}", m, s)
                } else {
                    "SNOOZE DISABLED".to_string()
                };
                (l1.to_string(), l2.to_string(), l3)
            }
            UiMode::OnBreak { secs_left } => {
                let m = secs_left / 60;
                let s = secs_left % 60;
                (
                    "INTERLUDE".to_string(),
                    format!("TIME LEFT {}:{:02}", m, s),
                    " ".to_string(),
                )
            }
            UiMode::BreakFinished => (
                "INTERLUDE COMPLETE".to_string(),
                "ENTER: RETURN".to_string(),
                " ".to_string(),
            ),
        };

        draw_text_rgba(&mut bytes, w, h, 40, 60, &l1, white);
        draw_text_rgba(&mut bytes, w, h, 40, 80, &l2, white);
        draw_text_rgba(&mut bytes, w, h, 40, 100, &l3, white);

        // Create a shm pool and buffer each redraw (MVP).
        // Optimization later: reuse pool/buffer and only rewrite bytes.
        let fd = rustix::fs::memfd_create("interlude-frame", rustix::fs::MemfdFlags::CLOEXEC)
            .map_err(|e| anyhow!("memfd_create: {e}"))?;
        rustix::fs::ftruncate(&fd, size as u64).map_err(|e| anyhow!("ftruncate: {e}"))?;

        // mmap and copy bytes
        let mut map = unsafe { memmap2::MmapMut::map_mut(&std::fs::File::from(fd.as_fd())) }
            .map_err(|e| anyhow!("mmap: {e}"))?;
        map[..].copy_from_slice(&bytes);
        map.flush().ok();

        let pool = shm.create_pool(fd.as_fd(), size as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            w as i32,
            h as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
            &qh,
            (),
        );

        {
            let s = &mut self.state.surfaces[idx];
            s.shm_pool = Some(pool);
            s.buffer = Some(buffer.clone());
            s.shm_bytes = bytes;
            s.stride = stride;
        }

        let s = &self.state.surfaces[idx];
        s.wl_surface.attach(Some(&buffer), 0, 0);
        s.wl_surface.damage_buffer(0, 0, w as i32, h as i32);
        s.wl_surface.commit();
    }
}

// ---------- Dispatch impls ----------

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if format != wl_keyboard::KeymapFormat::XkbV1 {
                    return;
                }
                // Read keymap string from fd
                use std::io::Read;
                let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
                let mut buf = vec![0u8; size as usize];
                if file.read_exact(&mut buf).is_ok() {
                    if let Ok(s) = std::str::from_utf8(&buf) {
                        if let Ok(keymap) = xkb::Keymap::new_from_string(
                            &state.xkb_context,
                            s,
                            xkb::KEYMAP_FORMAT_TEXT_V1,
                            xkb::COMPILE_NO_FLAGS,
                        ) {
                            state.xkb_state = xkb::State::new(&keymap).ok();
                            state.xkb_keymap = Some(keymap);
                        }
                    }
                }
            }
            wl_keyboard::Event::Key { key, state: kstate, .. } => {
                if kstate != wl_keyboard::KeyState::Pressed {
                    return;
                }
                let Some(xkbs) = &mut state.xkb_state else { return; };

                // Wayland keycodes are offset by 8 from evdev
                let sym = xkbs.key_get_one_sym(key + 8);

                // Decode minimal keys: Enter and 'z'
                // xkbcommon keysyms: Return = 0xff0d, z = 0x007a (lowercase)
                match sym {
                    0xff0d => {
                        let _ = state.tx_ui.send(UiEvent::PressEnter);
                    }
                    0x007a | 0x005a => {
                        let _ = state.tx_ui.send(UiEvent::PressZ);
                    }
                    _ => {}
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
                if let Some(xkbs) = &mut state.xkb_state {
                    xkbs.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ExtSessionLockSurfaceV1,
        event: <ExtSessionLockSurfaceV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            // Configure gives us width/height for each output surface
            wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::Event::Configure { serial, width, height } => {
                proxy.ack_configure(serial);

                for s in state.surfaces.iter_mut() {
                    if &s.lock_surface == proxy {
                        if width > 0 && height > 0 {
                            s.width = width;
                            s.height = height;
                            s.stride = (width as i32) * 4;
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

// Boilerplate: unused but required for compilation in some setups
impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &wl_output::WlOutput,
        _event: wl_output::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WlBuffer,
        _event: wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShmPool, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WlShmPool,
        _event: wayland_client::protocol::wl_shm_pool::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtSessionLockV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &ExtSessionLockV1,
        _event: wayland_protocols::ext::session_lock::v1::client::ext_session_lock_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtSessionLockManagerV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &ExtSessionLockManagerV1,
        _event: wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_dispatch!(State: [wl_keyboard::WlKeyboard: ()] => State);
wayland_client::delegate_dispatch!(State: [wl_output::WlOutput: ()] => State);
wayland_client::delegate_dispatch!(State: [wayland_client::protocol::wl_buffer::WlBuffer: ()] => State);
wayland_client::delegate_dispatch!(State: [wayland_client::protocol::wl_shm_pool::WlShmPool: ()] => State);
wayland_client::delegate_dispatch!(State: [ExtSessionLockSurfaceV1: ()] => State);
wayland_client::delegate_dispatch!(State: [ExtSessionLockV1: ()] => State);
wayland_client::delegate_dispatch!(State: [ExtSessionLockManagerV1: ()] => State);

