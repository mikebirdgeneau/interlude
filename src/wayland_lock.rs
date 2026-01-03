use anyhow::{Result, anyhow};
use crossbeam_channel::Sender;
use rustix::fd::IntoRawFd;
use std::os::fd::{AsFd, FromRawFd};

use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    backend::WaylandError,
    protocol::{
        wl_buffer, wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_keyboard, wl_output,
        wl_output::WlOutput, wl_pointer, wl_region::WlRegion, wl_registry, wl_seat::WlSeat,
        wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

use xkbcommon::xkb;

use crate::tiny_font::{draw_text_rgba_size, line_ascent_size, line_height_size, text_width_size};
use std::time::{Duration, Instant};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, TreeParsing};

#[derive(Debug, Clone, Copy)]
pub enum UiEvent {
    PressZ,
    PressEnter,
    PointerClick,
    AnyKey,
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

#[derive(Debug, Clone, Copy)]
pub struct UiColors {
    pub background: [u8; 4],
    pub foreground: [u8; 4],
}

#[derive(Clone)]
struct Icon {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

struct SurfaceCtx {
    _output: WlOutput,
    wl_surface: WlSurface,
    layer_surface: ZwlrLayerSurfaceV1,
    width: u32,
    height: u32,
    input_region: Option<WlRegion>,
    icon: Option<Icon>,
    small_icon: Option<Icon>,
    small_icon_size: u32,

    // SHM objects (recreated on resize/configure)
    shm_pool: Option<WlShmPool>,
    buffer: Option<WlBuffer>,
    shm_bytes: Vec<u8>,
    stride: i32,
}

#[derive(Debug, Clone)]
enum FadeState {
    None,
    In { start: Instant },
    Out { start: Instant },
}

struct State {
    _registry: Option<wl_registry::WlRegistry>,
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    seat: Option<WlSeat>,
    layer_shell: Option<ZwlrLayerShellV1>,
    icon_tree: Option<resvg::Tree>,

    outputs: Vec<WlOutput>,
    surfaces: Vec<SurfaceCtx>,

    overlay_active: bool,
    overlay_alpha: u8,
    fade: FadeState,
    input_captured: bool,
    desired_capture: bool,
    fade_in_complete: bool,
    text_alpha: u8,
    max_alpha: u8,
    colors: UiColors,

    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    xkb_context: xkb::Context,
    xkb_keymap: Option<xkb::Keymap>,
    xkb_state: Option<xkb::State>,

    ui_mode: UiMode,
    tx_ui: Sender<UiEvent>,
}

const FADE_IN_DURATION: Duration = Duration::from_secs(15);
const FADE_OUT_DURATION: Duration = Duration::from_millis(500);
const TEXT_FADE_IN_WINDOW: Duration = Duration::from_secs(3);
const ICON_SVG: &[u8] = include_bytes!("../assets/plant-2.svg");
const ICON_BASE_SIZE: u32 = 120;
const ICON_GAP: i32 = 20;

fn render_icon(tree: &resvg::Tree, size: u32) -> Option<Icon> {
    let mut pixmap = Pixmap::new(size, size)?;
    let sx = size as f32 / tree.size.width();
    let sy = size as f32 / tree.size.height();
    let mut pixmap_mut = pixmap.as_mut();
    tree.render(Transform::from_scale(sx, sy), &mut pixmap_mut);
    Some(Icon {
        width: size,
        height: size,
        rgba: pixmap.data().to_vec(),
    })
}

fn draw_icon_rgba(
    buf: &mut [u8],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    icon: &Icon,
    tint: [u8; 3],
    alpha_mul: u8,
) {
    for iy in 0..icon.height {
        for ix in 0..icon.width {
            let px = x + ix as i32;
            let py = y + iy as i32;
            if px < 0 || py < 0 || (px as u32) >= width || (py as u32) >= height {
                continue;
            }
            let src_idx = ((iy * icon.width + ix) * 4) as usize;
            let alpha = (icon.rgba[src_idx + 3] as u16 * alpha_mul as u16 / 255) as u8;
            if alpha == 0 {
                continue;
            }
            let dst_idx = ((py as u32 * width + px as u32) * 4) as usize;
            let inv = 255u16.saturating_sub(alpha as u16);
            let a = alpha as u16;
            buf[dst_idx] = ((tint[0] as u16 * a + buf[dst_idx] as u16 * inv) / 255) as u8;
            buf[dst_idx + 1] = ((tint[1] as u16 * a + buf[dst_idx + 1] as u16 * inv) / 255) as u8;
            buf[dst_idx + 2] = ((tint[2] as u16 * a + buf[dst_idx + 2] as u16 * inv) / 255) as u8;
            buf[dst_idx + 3] = 255;
        }
    }
}

impl Locker {
    pub fn new(tx_ui: Sender<UiEvent>, colors: UiColors) -> Result<Self> {
        let conn = Connection::connect_to_env()?;
        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();
        let registry = conn.display().get_registry(&qh, ());

        let icon_tree = {
            let opts = Options::default();
            let usvg_tree = resvg::usvg::Tree::from_data(ICON_SVG, &opts).ok();
            usvg_tree.map(|tree| resvg::Tree::from_usvg(&tree))
        };

        let mut state = State {
            _registry: Some(registry),
            compositor: None,
            shm: None,
            seat: None,
            layer_shell: None,
            icon_tree,
            outputs: vec![],
            surfaces: vec![],
            overlay_active: false,
            overlay_alpha: colors.background[3],
            fade: FadeState::None,
            input_captured: false,
            desired_capture: false,
            fade_in_complete: false,
            text_alpha: 255,
            max_alpha: colors.background[3],
            colors,
            keyboard: None,
            pointer: None,
            xkb_context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            xkb_keymap: None,
            xkb_state: None,
            ui_mode: UiMode::BreakDue {
                snooze_secs: 300,
                can_snooze: true,
            },
            tx_ui,
        };
        event_queue.roundtrip(&mut state)?;

        if state.compositor.is_none()
            || state.shm.is_none()
            || state.seat.is_none()
            || state.layer_shell.is_none()
        {
            return Err(anyhow!(
                "Missing required Wayland globals (compositor/shm/seat/layer_shell)"
            ));
        }

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
        if let Some(guard) = self.event_queue.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(err)) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(err) => return Err(anyhow!("wayland read: {err}")),
            }
        }
        self.event_queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }

    pub fn set_mode(&mut self, mode: UiMode) {
        self.state.ui_mode = mode;
        self.redraw_all();
    }

    pub fn start_fade_in(&mut self) {
        if matches!(self.state.fade, FadeState::In { .. }) {
            return;
        }
        self.state.fade = FadeState::In {
            start: Instant::now(),
        };
        self.state.overlay_alpha = 0;
        self.state.text_alpha = 0;
        self.state.fade_in_complete = false;
        self.set_input_capture(false);
        self.redraw_all();
    }

    pub fn start_fade_out(&mut self) {
        if matches!(self.state.fade, FadeState::Out { .. }) {
            return;
        }
        self.state.fade = FadeState::Out {
            start: Instant::now(),
        };
        self.state.overlay_alpha = self.state.max_alpha;
        self.state.text_alpha = 255;
        self.set_input_capture(false);
        self.redraw_all();
    }

    pub fn is_fading(&self) -> bool {
        !matches!(self.state.fade, FadeState::None)
    }

    pub fn take_fade_in_complete(&mut self) -> bool {
        if self.state.fade_in_complete {
            self.state.fade_in_complete = false;
            true
        } else {
            false
        }
    }

    pub fn ensure_input_capture(&mut self) {
        self.set_input_capture(true);
    }

    pub fn update_fade(&mut self) -> bool {
        let (alpha, done, finished_fade_out) = match self.state.fade.clone() {
            FadeState::None => return false,
            FadeState::In { start } => {
                let progress =
                    (Instant::now() - start).as_secs_f32() / FADE_IN_DURATION.as_secs_f32();
                let p = progress.clamp(0.0, 1.0);
                let alpha = (self.state.max_alpha as f32 * p).round() as u8;
                let text_start =
                    1.0 - (TEXT_FADE_IN_WINDOW.as_secs_f32() / FADE_IN_DURATION.as_secs_f32());
                let text_progress = if p <= text_start {
                    0.0
                } else {
                    (p - text_start) / (1.0 - text_start)
                };
                self.state.text_alpha = (self.state.colors.foreground[3] as f32
                    * text_progress.clamp(0.0, 1.0))
                .round() as u8;
                (alpha, p >= 1.0, false)
            }
            FadeState::Out { start } => {
                let progress =
                    (Instant::now() - start).as_secs_f32() / FADE_OUT_DURATION.as_secs_f32();
                let p = progress.clamp(0.0, 1.0);
                let alpha = (self.state.max_alpha as f32 * (1.0 - p)).round() as u8;
                self.state.text_alpha = ((self.state.colors.foreground[3] as u16 * alpha as u16)
                    / self.state.max_alpha as u16) as u8;
                (alpha, p >= 1.0, true)
            }
        };

        if alpha != self.state.overlay_alpha {
            self.state.overlay_alpha = alpha;
            self.redraw_all();
        }

        if done {
            self.state.fade = FadeState::None;
            if !finished_fade_out {
                self.state.fade_in_complete = true;
                self.state.text_alpha = 255;
                self.set_input_capture(true);
            }
        }

        done && finished_fade_out
    }

    fn set_input_capture(&mut self, enable: bool) {
        if self.state.input_captured == enable {
            return;
        }
        self.state.input_captured = enable;
        self.state.desired_capture = enable;
        if !self.state.overlay_active {
            return;
        }

        let interactivity = if enable {
            zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive
        } else {
            zwlr_layer_surface_v1::KeyboardInteractivity::None
        };

        let compositor = match self.state.compositor.clone() {
            Some(compositor) => compositor,
            None => return,
        };

        let qh = self.event_queue.handle();

        for surface in self.state.surfaces.iter_mut() {
            surface
                .layer_surface
                .set_keyboard_interactivity(interactivity);
            if enable {
                surface.wl_surface.set_input_region(None);
                surface.input_region = None;
            } else {
                let region = compositor.create_region(&qh, ());
                surface.wl_surface.set_input_region(Some(&region));
                surface.input_region = Some(region);
            }
            surface.wl_surface.commit();
        }
    }

    pub fn is_locked(&self) -> bool {
        self.state.overlay_active
    }

    pub fn lock(&mut self) -> Result<()> {
        if self.is_locked() {
            return Ok(());
        }
        let qh = self.event_queue.handle();

        // create a surface per output
        self.state.surfaces.clear();
        self.state.input_captured = false;
        self.state.desired_capture = false;
        let compositor = self.state.compositor.clone().unwrap();
        let layer_shell = self.state.layer_shell.clone().unwrap();

        for out in self.state.outputs.iter().cloned() {
            let wl_surface = compositor.create_surface(&qh, ());
            let layer_surface = layer_shell.get_layer_surface(
                &wl_surface,
                Some(&out),
                Layer::Overlay,
                "interlude".to_string(),
                &qh,
                (),
            );
            layer_surface.set_anchor(
                zwlr_layer_surface_v1::Anchor::Top
                    | zwlr_layer_surface_v1::Anchor::Bottom
                    | zwlr_layer_surface_v1::Anchor::Left
                    | zwlr_layer_surface_v1::Anchor::Right,
            );
            let interactivity = if self.state.desired_capture {
                zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive
            } else {
                zwlr_layer_surface_v1::KeyboardInteractivity::None
            };
            layer_surface.set_keyboard_interactivity(interactivity);
            layer_surface.set_exclusive_zone(-1);
            layer_surface.set_size(0, 0);
            let input_region = if !self.state.desired_capture {
                let region = compositor.create_region(&qh, ());
                wl_surface.set_input_region(Some(&region));
                Some(region)
            } else {
                wl_surface.set_input_region(None);
                None
            };
            wl_surface.commit();

            // placeholder until configure
            let (w, h) = (0u32, 0u32);

            self.state.surfaces.push(SurfaceCtx {
                _output: out,
                wl_surface,
                layer_surface,
                width: w,
                height: h,
                input_region,
                icon: None,
                small_icon: None,
                small_icon_size: 0,
                shm_pool: None,
                buffer: None,
                shm_bytes: vec![],
                stride: (w as i32) * 4,
            });
        }

        // roundtrip so we receive configure sizes
        self.roundtrip()?;

        self.state.overlay_active = true;
        self.redraw_all();
        Ok(())
    }

    pub fn unlock(&mut self) {
        for surface in self.state.surfaces.drain(..) {
            surface.layer_surface.destroy();
            surface.wl_surface.destroy();
        }
        self.state.overlay_active = false;
        self.state.input_captured = false;
        self.state.desired_capture = false;
    }

    fn redraw_all(&mut self) {
        for i in 0..self.state.surfaces.len() {
            if let Err(err) = self.redraw_surface(i) {
                eprintln!("redraw error: {err}");
            }
        }
    }

    fn redraw_surface(&mut self, idx: usize) -> Result<()> {
        let qh = self.event_queue.handle();
        let shm = match self.state.shm.clone() {
            Some(s) => s,
            None => return Ok(()),
        };

        let (w, h) = {
            let s = &self.state.surfaces[idx];
            (s.width, s.height)
        };

        if w == 0 || h == 0 {
            return Ok(());
        }

        let stride = (w as i32) * 4;
        let size = (stride as usize) * (h as usize);

        let white = [
            self.state.colors.foreground[0],
            self.state.colors.foreground[1],
            self.state.colors.foreground[2],
            self.state.text_alpha,
        ];
        #[derive(Clone, Copy)]
        enum LineAnchor {
            Center,
            CenterOnColon,
        }

        struct LineSpec {
            text: String,
            size: f32,
            alpha: f32,
            anchor: LineAnchor,
        }

        let base_size = (w.min(h) as f32 / 16.0).clamp(42.0, 110.0);
        let large_size = (base_size * 1.35).clamp(56.0, 150.0);
        let small_size = (base_size * 0.7).clamp(28.0, 80.0);

        let lines = match &self.state.ui_mode {
            UiMode::BreakDue {
                snooze_secs,
                can_snooze,
            } => {
                let l1 = "BREAK STARTING".to_string();
                let l2 = if *can_snooze {
                    let m = snooze_secs / 60;
                    let s = snooze_secs % 60;
                    format!("Snooze: z/Esc {}:{:02}", m, s)
                } else {
                    "Snooze disabled".to_string()
                };
                vec![
                    LineSpec {
                        text: l1,
                        size: base_size,
                        alpha: 1.0,
                        anchor: LineAnchor::Center,
                    },
                    LineSpec {
                        text: l2,
                        size: small_size,
                        alpha: 0.65,
                        anchor: LineAnchor::Center,
                    },
                ]
            }
            UiMode::OnBreak { secs_left } => {
                let m = secs_left / 60;
                let s = secs_left % 60;
                vec![
                    LineSpec {
                        text: format!("{:02}:{:02}", m, s),
                        size: large_size,
                        alpha: 1.0,
                        anchor: LineAnchor::CenterOnColon,
                    },
                    LineSpec {
                        text: "Snooze: z/Esc".to_string(),
                        size: small_size,
                        alpha: 0.65,
                        anchor: LineAnchor::Center,
                    },
                ]
            }
            UiMode::BreakFinished => vec![
                LineSpec {
                    text: "Break Complete.".to_string(),
                    size: base_size,
                    alpha: 1.0,
                    anchor: LineAnchor::Center,
                },
                LineSpec {
                    text: "Press any key to continue".to_string(),
                    size: small_size,
                    alpha: 0.65,
                    anchor: LineAnchor::Center,
                },
            ],
        };

        let icon_size = {
            let mut size = (w.min(h) / 6).max(ICON_BASE_SIZE);
            size = size.min(ICON_BASE_SIZE * 2);
            size
        };

        let (icon, small_icon) = {
            let s = &mut self.state.surfaces[idx];
            let needs_icon = s
                .icon
                .as_ref()
                .map(|icon| icon.width != icon_size)
                .unwrap_or(true);
            if needs_icon {
                if let Some(tree) = &self.state.icon_tree {
                    s.icon = render_icon(tree, icon_size);
                }
            }

            let small_icon = if matches!(self.state.fade, FadeState::In { .. }) {
                let small_size = (icon_size / 3).max(24);
                let needs_small = s.small_icon_size != small_size || s.small_icon.is_none();
                if needs_small {
                    if let Some(tree) = &self.state.icon_tree {
                        s.small_icon = render_icon(tree, small_size);
                        s.small_icon_size = small_size;
                    }
                }
                s.small_icon.clone()
            } else {
                None
            };

            (s.icon.clone(), small_icon)
        };

        let icon_height = icon
            .as_ref()
            .map(|icon| icon.height as i32)
            .unwrap_or(0);

        // Dim background: mostly opaque black
        let bg_alpha = 255;
        let mut bytes = {
            let s = &mut self.state.surfaces[idx];
            s.shm_bytes.resize(size, 0u8);
            std::mem::take(&mut s.shm_bytes)
        };
        for px in bytes.chunks_exact_mut(4) {
            px.copy_from_slice(&[
                self.state.colors.background[0],
                self.state.colors.background[1],
                self.state.colors.background[2],
                bg_alpha,
            ]);
        }

        let text_height: i32 = lines.iter().map(|line| line_height_size(line.size)).sum();
        let total_height = icon_height + if icon_height > 0 { ICON_GAP } else { 0 } + text_height;
        let base_y = ((h as i32 - total_height) / 2).max(0);

        let tint = [
            self.state.colors.foreground[0],
            self.state.colors.foreground[1],
            self.state.colors.foreground[2],
        ];

        if let Some(icon) = icon.as_ref() {
            let icon_x = ((w as i32 - icon.width as i32) / 2).max(0);
            if self.state.text_alpha > 0 {
                draw_icon_rgba(&mut bytes, w, h, icon_x, base_y, icon, tint, self.state.text_alpha);
            }
        }

        let text_start_y = base_y + icon_height + if icon_height > 0 { ICON_GAP } else { 0 };
        let mut line_y = text_start_y;
        for line in &lines {
            let ascent = line_ascent_size(line.size);
            let base_x = match line.anchor {
                LineAnchor::Center => {
                    let line_width = text_width_size(&line.text, line.size);
                    ((w as i32 - line_width) / 2).max(0)
                }
                LineAnchor::CenterOnColon => {
                    if let Some(idx) = line.text.find(':') {
                        let (left, _) = line.text.split_at(idx);
                        let left_width = text_width_size(left, line.size);
                        let colon_width = text_width_size(":", line.size);
                        ((w as i32 / 2) - left_width - (colon_width / 2)).max(0)
                    } else {
                        let line_width = text_width_size(&line.text, line.size);
                        ((w as i32 - line_width) / 2).max(0)
                    }
                }
            };
            let alpha = ((self.state.text_alpha as f32) * line.alpha).round() as u8;
            let rgba = [white[0], white[1], white[2], alpha];
            draw_text_rgba_size(&mut bytes, w, h, base_x, line_y + ascent, &line.text, rgba, line.size);
            line_y += line_height_size(line.size);
        }

        if let Some(icon) = small_icon.as_ref() {
            let pad = 20;
            let x = w as i32 - icon.width as i32 - pad;
            let y = h as i32 - icon.height as i32 - pad;
            draw_icon_rgba(&mut bytes, w, h, x, y, icon, tint, 255);
        }

        let fade = self.state.overlay_alpha as u16;
        for px in bytes.chunks_exact_mut(4) {
            px[0] = ((px[0] as u16 * fade) / 255) as u8;
            px[1] = ((px[1] as u16 * fade) / 255) as u8;
            px[2] = ((px[2] as u16 * fade) / 255) as u8;
            px[3] = fade as u8;
        }

        // Create a shm pool and buffer each redraw (MVP).
        // Optimization later: reuse pool/buffer and only rewrite bytes.
        let fd = rustix::fs::memfd_create("interlude-frame", rustix::fs::MemfdFlags::CLOEXEC)
            .map_err(|e| anyhow!("memfd_create: {e}"))?;
        rustix::fs::ftruncate(&fd, size as u64).map_err(|e| anyhow!("ftruncate: {e}"))?;
        let raw_fd = fd.into_raw_fd();
        let file = unsafe { std::fs::File::from_raw_fd(raw_fd) };

        // mmap and copy bytes
        let mut map =
            unsafe { memmap2::MmapMut::map_mut(&file) }.map_err(|e| anyhow!("mmap: {e}"))?;
        map[..].copy_from_slice(&bytes);
        map.flush().ok();

        let pool = shm.create_pool(file.as_fd(), size as i32, &qh, ());
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
            s.shm_bytes = bytes;
            s.shm_pool = Some(pool);
            s.buffer = Some(buffer.clone());
            s.stride = stride;
        }

        let s = &self.state.surfaces[idx];
        s.wl_surface.attach(Some(&buffer), 0, 0);
        s.wl_surface.damage_buffer(0, 0, w as i32, h as i32);
        s.wl_surface.commit();
        Ok(())
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
                if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    return;
                }
                // Read keymap string from fd
                use std::io::Read;
                let mut file = std::fs::File::from(fd);
                let mut buf = vec![0u8; size as usize];
                if file.read_exact(&mut buf).is_ok() {
                    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
                    if let Ok(s) = std::str::from_utf8(&buf[..end]) {
                        if let Some(keymap) = xkb::Keymap::new_from_string(
                            &state.xkb_context,
                            s.to_string(),
                            xkb::KEYMAP_FORMAT_TEXT_V1,
                            xkb::COMPILE_NO_FLAGS,
                        ) {
                            state.xkb_state = Some(xkb::State::new(&keymap));
                            state.xkb_keymap = Some(keymap);
                        }
                    }
                }
            }
            wl_keyboard::Event::Enter { .. } => {}
            wl_keyboard::Event::Leave { .. } => {}
            wl_keyboard::Event::Key {
                key, state: kstate, ..
            } => {
                if kstate != WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    return;
                }
                if let Some(xkbs) = &mut state.xkb_state {
                    // Wayland keycodes are offset by 8 from evdev
                    let sym = xkbs.key_get_one_sym((key + 8).into());

                    // Decode minimal keys: Enter, 'z', and Escape (snooze)
                    // xkbcommon keysyms: Return = 0xff0d, Escape = 0xff1b, z = 0x007a
                    match sym.raw() {
                        0xff0d => {
                            let _ = state.tx_ui.send(UiEvent::PressEnter);
                        }
                        0xff1b => {
                            let _ = state.tx_ui.send(UiEvent::PressZ);
                        }
                        0x007a | 0x005a => {
                            let _ = state.tx_ui.send(UiEvent::PressZ);
                        }
                        _ => {}
                    }
                } else {
                    // Fallback to common evdev keycodes if no keymap yet.
                    match key {
                        1 => {
                            let _ = state.tx_ui.send(UiEvent::PressZ);
                        }
                        28 => {
                            let _ = state.tx_ui.send(UiEvent::PressEnter);
                        }
                        44 => {
                            let _ = state.tx_ui.send(UiEvent::PressZ);
                        }
                        _ => {}
                    }
                }
                let _ = state.tx_ui.send(UiEvent::AnyKey);
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(xkbs) = &mut state.xkb_state {
                    xkbs.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: <ZwlrLayerSurfaceV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                proxy.ack_configure(serial);

                for s in state.surfaces.iter_mut() {
                    if &s.layer_surface == proxy {
                        if width > 0 {
                            s.width = width;
                        }
                        if height > 0 {
                            s.height = height;
                        }
                        if s.width > 0 && s.height > 0 {
                            s.stride = (s.width as i32) * 4;
                            if state.overlay_active {
                                if state.desired_capture {
                                    s.wl_surface.set_input_region(None);
                                    s.input_region = None;
                                } else if let Some(compositor) = state.compositor.clone() {
                                    let region = compositor.create_region(_qh, ());
                                    s.wl_surface.set_input_region(Some(&region));
                                    s.input_region = Some(region);
                                }
                                s.wl_surface.commit();
                            }
                        }
                        break;
                    }
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.overlay_active = false;
                state.surfaces.clear();
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" if state.compositor.is_none() => {
                    let ver = version.min(WlCompositor::interface().version);
                    state.compositor = Some(proxy.bind(name, ver, qh, ()));
                }
                "wl_shm" if state.shm.is_none() => {
                    let ver = version.min(WlShm::interface().version);
                    state.shm = Some(proxy.bind(name, ver, qh, ()));
                }
                "wl_seat" if state.seat.is_none() => {
                    let ver = version.min(WlSeat::interface().version);
                    state.seat = Some(proxy.bind(name, ver, qh, ()));
                }
                "wl_output" => {
                    let ver = version.min(WlOutput::interface().version);
                    let out = proxy.bind(name, ver, qh, ());
                    state.outputs.push(out);
                }
                "zwlr_layer_shell_v1" if state.layer_shell.is_none() => {
                    let ver = version.min(ZwlrLayerShellV1::interface().version);
                    state.layer_shell = Some(proxy.bind(name, ver, qh, ()));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { .. } => {}
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

impl Dispatch<ZwlrLayerShellV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrLayerShellV1,
        _event: wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WlCompositor,
        _event: wayland_client::protocol::wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShm, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WlShm,
        _event: wayland_client::protocol::wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &WlSeat,
        event: wayland_client::protocol::wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wayland_client::protocol::wl_seat::Event::Capabilities { capabilities } => {
                let has_keyboard = match capabilities {
                    WEnum::Value(caps) => {
                        caps.contains(wayland_client::protocol::wl_seat::Capability::Keyboard)
                    }
                    WEnum::Unknown(_) => false,
                };
                let has_pointer = match capabilities {
                    WEnum::Value(caps) => {
                        caps.contains(wayland_client::protocol::wl_seat::Capability::Pointer)
                    }
                    WEnum::Unknown(_) => false,
                };

                if has_keyboard && state.keyboard.is_none() {
                    state.keyboard = Some(proxy.get_keyboard(qh, ()));
                } else if !has_keyboard {
                    if let Some(kbd) = state.keyboard.take() {
                        kbd.release();
                    }
                    state.xkb_state = None;
                    state.xkb_keymap = None;
                }

                if has_pointer && state.pointer.is_none() {
                    state.pointer = Some(proxy.get_pointer(qh, ()));
                } else if !has_pointer {
                    if let Some(ptr) = state.pointer.take() {
                        ptr.release();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSurface, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WlSurface,
        _event: wayland_client::protocol::wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlRegion, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegion,
        _event: wayland_client::protocol::wl_region::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Button {
                state: btn_state, ..
            } => {
                if btn_state == WEnum::Value(wl_pointer::ButtonState::Pressed) {
                    let _ = state.tx_ui.send(UiEvent::PointerClick);
                }
            }
            _ => {}
        }
    }
}
