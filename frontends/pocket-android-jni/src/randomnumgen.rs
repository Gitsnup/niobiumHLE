//! Android compatibility renderer for tectrasystems.org Random Number
//! Generator (`RandomNumGen v1.2`).
//!
//! Android has no host CLR to run .NET Compact Framework images, so —
//! as with vAlienAttack — the application is recreated as a native Rust
//! renderer driven by the session input stream. Everything here mirrors
//! the guest's decompiled IL, not an interpretation of it:
//!
//! * `.ctor` places `Minimum:` at (10,10) with its NumericUpDown at
//!   (90,10) sized `(screen.Width - 100, 20)` bounded [0, 999999],
//!   `Maximum:` at (10,40) with its control at (90,40) bounded
//!   [1, 1000000], the log ListBox at (10,70) sized `(Width - 20, 140)`
//!   and the credit label at (10,220). The menu carries Generate and
//!   Clear in that order.
//! * `generateBtn_Click` parses both box texts, asks
//!   `new Random().Next(min, max + 1)` — inclusive on both ends — and
//!   appends `"{min}-{max}: {n}"` to the log.
//! * `clearBtn_Click` clears the whole log when nothing is selected and
//!   otherwise removes only the selected row.

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use pocket_core::kernel::{Framebuffer, InputEvent};

use crate::managed_game::{draw_rect, draw_text, rgb565, ManagedRenderer};
use crate::runner::{InputCommand, SessionState};

/// An `x, y, width, height` rectangle in logical screen coordinates.
type Rect = (i32, i32, i32, i32);

const MENU_HEIGHT: i32 = 20;

const MENU_BG: u16 = rgb565(236, 236, 226);
const CLIENT_BG: u16 = rgb565(212, 208, 200);
const CONTROL_BG: u16 = rgb565(255, 255, 255);
const LIST_BG: u16 = rgb565(255, 255, 255);
const BLACK: u16 = rgb565(0, 0, 0);
const SELECT_BG: u16 = rgb565(0, 0, 128);
const SELECT_FG: u16 = rgb565(255, 255, 255);
const ARROW_BG: u16 = rgb565(212, 208, 200);

/// Menu hit rows, generous because a stylus tap is never pixel-exact.
const GENERATE_MENU_MAX_X: i32 = 60;
const CLEAR_MENU_MAX_X: i32 = 110;

struct RandomNumGenApp {
    screen: (u32, u32),
    min: i64,
    max: i64,
    log: Vec<String>,
    selected: Option<usize>,
    rng: XorShift64,
}

impl RandomNumGenApp {
    fn new(screen: (u32, u32)) -> Result<Self> {
        // `maxBox` is constructed with Minimum = Decimal.One, so its
        // value opens at 1 even though nothing sets it explicitly.
        Ok(Self {
            screen,
            min: 0,
            max: 1,
            log: Vec::new(),
            selected: None,
            rng: XorShift64::seeded_from_clock(),
        })
    }

    fn logical_x(&self, x: u16) -> i32 {
        (x as u32 * 240 / self.screen.0.max(1)) as i32
    }

    fn logical_y(&self, y: u16) -> i32 {
        (y as u32 * 320 / self.screen.1.max(1)) as i32
    }

    /// Screen-space geometry of a NumericUpDown at client `top`: the box
    /// rect plus its up and down arrow buttons at the right edge.
    fn spin_geometry(&self, top: i32) -> (Rect, Rect, Rect) {
        let y = MENU_HEIGHT + top;
        ((90, y, 140, 20), (212, y + 1, 17, 9), (212, y + 10, 17, 9))
    }

    fn bump_min(&mut self, delta: i64) {
        // minBox: Minimum = Decimal.Zero, Maximum = Decimal(999999).
        self.min = (self.min + delta).clamp(0, 999_999);
    }

    fn bump_max(&mut self, delta: i64) {
        // maxBox: Minimum = Decimal.One, Maximum = Decimal(1000000).
        self.max = (self.max + delta).clamp(1, 1_000_000);
    }

    /// `generateBtn_Click`: `Random.Next(min, max + 1)` and a
    /// `"{min}-{max}: {n}"` row. The guest would throw
    /// ArgumentOutOfRangeException for min > max; the renderer cannot
    /// crash the session, so it leaves the log untouched instead.
    fn generate(&mut self) {
        if self.min > self.max {
            return;
        }
        let n = self.rng.next_in_range(self.min, self.max);
        self.log.push(format!("{}-{}: {}", self.min, self.max, n));
        self.selected = None;
    }

    /// `clearBtn_Click`: nothing selected clears everything, otherwise
    /// only the selected row goes.
    fn clear(&mut self) {
        match self.selected {
            None => self.log.clear(),
            Some(index) => {
                if index < self.log.len() {
                    self.log.remove(index);
                }
                self.selected = None;
            }
        }
    }

    fn list_rect(&self) -> (i32, i32, i32, i32) {
        (10, MENU_HEIGHT + 70, 220, 140)
    }

    /// Rows the 140px list shows once full: the newest ones, because
    /// generate selects the last row before deselecting it.
    fn visible_rows(&self) -> (usize, usize) {
        const CAPACITY: usize = 9;
        if self.log.len() <= CAPACITY {
            (0, self.log.len())
        } else {
            (self.log.len() - CAPACITY, self.log.len())
        }
    }
}

impl ManagedRenderer for RandomNumGenApp {
    fn handle(&mut self, event: InputEvent) {
        match event {
            InputEvent::PointerDown { x, y } => {
                let x = self.logical_x(x);
                let y = self.logical_y(y);
                if y < MENU_HEIGHT {
                    if x < GENERATE_MENU_MAX_X {
                        self.generate();
                    } else if x < CLEAR_MENU_MAX_X {
                        self.clear();
                    }
                    return;
                }
                let (_, min_up, min_down) = self.spin_geometry(10);
                let (_, max_up, max_down) = self.spin_geometry(40);
                if x >= min_up.0 {
                    if min_up.1 <= y && y < min_up.1 + min_up.3 {
                        self.bump_min(1);
                    } else if min_down.1 <= y && y < min_down.1 + min_down.3 {
                        self.bump_min(-1);
                    } else if max_up.1 <= y && y < max_up.1 + max_up.3 {
                        self.bump_max(1);
                    } else if max_down.1 <= y && y < max_down.1 + max_down.3 {
                        self.bump_max(-1);
                    }
                    return;
                }
                let (lx, ly, lw, lh) = self.list_rect();
                if lx <= x && x < lx + lw && ly <= y && y < ly + lh {
                    let row = ((y - ly - 4) / 14).max(0) as usize;
                    let (first, last) = self.visible_rows();
                    let index = first + row;
                    self.selected = if index < last { Some(index) } else { None };
                }
            }
            InputEvent::PointerMove { .. } | InputEvent::PointerUp { .. } => {}
            InputEvent::KeyDown { vk } => match vk {
                // The guest reaches generate only through the menu; Enter
                // is the keyboard path to the same handler.
                0x0d | 0x20 => self.generate(),
                _ => {}
            },
            InputEvent::KeyUp { .. } => {}
        }
    }

    fn tick(&mut self) {}

    fn render(&mut self, framebuffer: &mut Framebuffer) {
        framebuffer.fill(CLIENT_BG);
        let sx = framebuffer.width as f32 / 240.0;
        let sy = framebuffer.height as f32 / 320.0;

        // Menu bar — the toolbar the launch was missing.
        draw_rect(framebuffer, 0, 0, 240, MENU_HEIGHT, MENU_BG, (sx, sy));
        draw_text(framebuffer, 8, 7, "Generate", BLACK, (sx, sy));
        draw_text(framebuffer, 72, 7, "Clear", BLACK, (sx, sy));

        // Labels. The guest's Label size is 70x20; text sits near the top.
        draw_text(
            framebuffer,
            10,
            MENU_HEIGHT + 14,
            "Minimum:",
            BLACK,
            (sx, sy),
        );
        draw_text(
            framebuffer,
            10,
            MENU_HEIGHT + 44,
            "Maximum:",
            BLACK,
            (sx, sy),
        );

        for (top, value) in [(10, self.min), (40, self.max)] {
            let (rect, up, down) = self.spin_geometry(top);
            let (bx, by, bw, bh) = rect;
            draw_rect(framebuffer, bx, by, bw, bh, CONTROL_BG, (sx, sy));
            draw_rect(framebuffer, bx, by, bw, 1, BLACK, (sx, sy));
            draw_rect(framebuffer, bx, by + bh - 1, bw, 1, BLACK, (sx, sy));
            draw_rect(framebuffer, bx, by, 1, bh, BLACK, (sx, sy));
            draw_rect(framebuffer, bx + bw - 1, by, 1, bh, BLACK, (sx, sy));
            draw_text(
                framebuffer,
                bx + 6,
                by + 7,
                &value.to_string(),
                BLACK,
                (sx, sy),
            );
            // Up/down spinners as small triangles on their own buttons
            // at the right edge of the box.
            draw_rect(framebuffer, up.0, up.1, up.2, up.3, ARROW_BG, (sx, sy));
            draw_rect(
                framebuffer,
                down.0,
                down.1,
                down.2,
                down.3,
                ARROW_BG,
                (sx, sy),
            );
            let mid = up.0 + up.2 / 2;
            for row in 0..3 {
                draw_rect(
                    framebuffer,
                    mid - row,
                    up.1 + 2 + row,
                    row * 2 + 1,
                    1,
                    BLACK,
                    (sx, sy),
                );
            }
            for row in 0..3 {
                draw_rect(
                    framebuffer,
                    mid - (2 - row),
                    down.1 + 2 + row,
                    (2 - row) * 2 + 1,
                    1,
                    BLACK,
                    (sx, sy),
                );
            }
        }

        // Log list box.
        let (lx, ly, lw, lh) = self.list_rect();
        draw_rect(framebuffer, lx, ly, lw, lh, LIST_BG, (sx, sy));
        draw_rect(framebuffer, lx, ly, lw, 1, BLACK, (sx, sy));
        draw_rect(framebuffer, lx, ly + lh - 1, lw, 1, BLACK, (sx, sy));
        draw_rect(framebuffer, lx, ly, 1, lh, BLACK, (sx, sy));
        draw_rect(framebuffer, lx + lw - 1, ly, 1, lh, BLACK, (sx, sy));
        let (first, last) = self.visible_rows();
        for (row, index) in (first..last).enumerate() {
            let y = ly + 4 + row as i32 * 14;
            if self.selected == Some(index) {
                draw_rect(framebuffer, lx + 2, y - 2, lw - 4, 13, SELECT_BG, (sx, sy));
                draw_text(
                    framebuffer,
                    lx + 4,
                    y,
                    &self.log[index],
                    SELECT_FG,
                    (sx, sy),
                );
            } else {
                draw_text(framebuffer, lx + 4, y, &self.log[index], BLACK, (sx, sy));
            }
        }

        // Credit label, centred across its 220px width.
        let credit = "Version 1.2 - by John Spahr";
        let width = credit.chars().count() as i32 * 6;
        draw_text(
            framebuffer,
            10 + (220 - width).max(0) / 2,
            MENU_HEIGHT + 228,
            credit,
            BLACK,
            (sx, sy),
        );

        framebuffer.mark_dirty();
    }
}

/// `Random` stands in. The guest seeds a fresh `System.Random` per
/// generate call; a clock-seeded xorshift64* per run keeps results
/// uncorrelated across sessions while staying dependency-free.
struct XorShift64(u64);

impl XorShift64 {
    fn seeded_from_clock() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        Self(nanos | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// `Random.Next(min, max + 1)` — inclusive on both ends.
    fn next_in_range(&mut self, min: i64, max: i64) -> i64 {
        if max <= min {
            return min;
        }
        let span = (max - min + 1) as u64;
        min + (self.next_u64() % span) as i64
    }
}

pub(crate) fn run(
    state: &Arc<SessionState>,
    input_rx: Receiver<InputCommand>,
    screen: (u32, u32),
) -> String {
    crate::managed_game::run_renderer(
        RandomNumGenApp::new(screen),
        "RandomNumGen",
        Path::new("RandomNumGen v1.2.exe"),
        state,
        input_rx,
        screen,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(app: &mut RandomNumGenApp, x: i32, y: i32) {
        app.handle(InputEvent::PointerDown {
            x: x as u16,
            y: y as u16,
        });
        app.handle(InputEvent::PointerUp {
            x: x as u16,
            y: y as u16,
        });
    }

    fn pixel(framebuffer: &mut Framebuffer, x: i32, y: i32) -> u16 {
        let stride = framebuffer.stride_bytes() as usize;
        let offset = y as usize * stride + x as usize * 2;
        let bytes = framebuffer.pixels_mut();
        u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
    }

    /// The launch complaint: without this renderer the menu bar with
    /// Generate/Clear never appeared on Android. The first frame must
    /// show it, and Generate must be reachable through it.
    #[test]
    fn first_frame_shows_the_menu_bar_and_generate_tap_produces_a_row() {
        let mut app = RandomNumGenApp::new((240, 320)).expect("renderer must initialize");
        let mut framebuffer = Framebuffer::new(240, 320);
        app.render(&mut framebuffer);
        assert!(!framebuffer.is_all_black());
        // Menu bar background, not the client backdrop.
        assert_eq!(pixel(&mut framebuffer, 30, 10), MENU_BG);
        // Defaults 0..=1: one row, value inside the range.
        tap(&mut app, 30, 10);
        assert_eq!(app.log.len(), 1);
        assert!(app.log[0].starts_with("0-1: "));
        let value: i64 = app.log[0]["0-1: ".len()..].parse().expect("numeric result");
        assert!((0..=1).contains(&value));
    }

    /// The up/down spinners move the parsed bounds the generate handler
    /// reads, within the decompiled [0,999999] and [1,1000000] limits.
    #[test]
    fn spinners_move_min_and_max_within_the_decompiled_bounds() {
        let mut app = RandomNumGenApp::new((240, 320)).expect("renderer must initialize");
        // maxBox opens at its Minimum, 1.
        assert_eq!(app.max, 1);
        // Down below the minimum does nothing.
        tap(&mut app, 220, 20 + 10 + 15);
        assert_eq!(app.max, 1);
        // Up raises it.
        tap(&mut app, 220, 20 + 40 + 5);
        assert_eq!(app.max, 2);
        // minBox down stops at 0.
        tap(&mut app, 220, 20 + 10 + 15);
        assert_eq!(app.min, 0);
        tap(&mut app, 220, 20 + 10 + 5);
        assert_eq!(app.min, 1);
        // Generate inside [1, 2].
        tap(&mut app, 30, 10);
        assert!(app.log[0].starts_with("1-2: "));
        let value: i64 = app.log[0]["1-2: ".len()..].parse().expect("numeric result");
        assert!((1..=2).contains(&value));
    }

    /// `clearBtn_Click`: with no selection it empties the log; a tapped
    /// row is removed on its own first.
    #[test]
    fn clear_empties_the_log_and_removes_only_a_selected_row() {
        let mut app = RandomNumGenApp::new((240, 320)).expect("renderer must initialize");
        for _ in 0..3 {
            tap(&mut app, 30, 10);
        }
        assert_eq!(app.log.len(), 3);
        // Select the second row: list top at 20+70=90, text rows every
        // 14px starting 4px in; row 1 is centred around y=108.
        tap(&mut app, 100, 108);
        assert_eq!(app.selected, Some(1));
        tap(&mut app, 90, 10);
        assert_eq!(app.log.len(), 2);
        // No selection now: Clear wipes the rest.
        assert_eq!(app.selected, None);
        tap(&mut app, 90, 10);
        assert!(app.log.is_empty());
    }

    /// min > max throws in the guest; the renderer skips the row
    /// instead of taking the session down.
    #[test]
    fn generating_with_min_above_max_leaves_the_log_untouched() {
        let mut app = RandomNumGenApp::new((240, 320)).expect("renderer must initialize");
        app.min = 5;
        app.max = 1;
        app.generate();
        assert!(app.log.is_empty());
    }
}

#[ignore]
#[test]
fn visual_frame_dump() {
    let mut app = RandomNumGenApp::new((240, 320)).unwrap();
    let mut fb = Framebuffer::new(240, 320);
    let mut snap = |app: &mut RandomNumGenApp, name: &str| {
        app.render(&mut fb);
        std::fs::write(format!("/tmp/rng-android-{name}.ppm"), fb.snapshot_ppm()).unwrap();
    };
    snap(&mut app, "startup");
    app.handle(InputEvent::PointerDown { x: 30, y: 6 });
    app.handle(InputEvent::PointerUp { x: 30, y: 6 });
    app.handle(InputEvent::PointerDown { x: 30, y: 6 });
    snap(&mut app, "two-generates");
    for _ in 0..3 {
        app.handle(InputEvent::PointerDown { x: 220, y: 61 });
        app.handle(InputEvent::PointerUp { x: 220, y: 61 });
    }
    app.handle(InputEvent::PointerDown { x: 30, y: 6 });
    app.handle(InputEvent::PointerUp { x: 30, y: 6 });
    snap(&mut app, "max-raised-to-4-and-generated");
    app.handle(InputEvent::PointerDown { x: 100, y: 100 });
    app.handle(InputEvent::PointerDown { x: 72, y: 6 });
    snap(&mut app, "after-clear");
}
