//! Frame tests for the main and settings windows: what a frame draws, and what it costs.
//!
//! The windows run on GPUI's test platform with the real DirectWrite text system, so a frame
//! lays out, shapes text and paints as on screen; only the GPU is left out. The rooms are made
//! up and the controller is detached (it starts no capture, discovery or firewall check). The
//! windows read, never write, two settings of this machine: the Windows accent color and
//! whether Zephon starts with Windows.
//!
//! A frame's cost is checked against a reference frame drawn in the same app: the two are timed
//! in turn, and the median of the per-pair ratios is compared with a limit. Other work on the
//! machine then slows both sides of a pair alike, so a slow or busy machine does not fail the
//! test, but a layout that grows several times more costly does. The timing tests run one at
//! a time (see [`TIMING`]). The times themselves are printed, not asserted: run
//! `cargo test --locked --release ui::perf -- --nocapture` to check them against the targets in
//! AGENTS.md.
//!
//! The frames that matter (docs/ARCHITECTURE.md, "UI responsiveness"):
//! - a full frame: GPUI draws one, with nothing reused, on each step of a window drag (every
//!   `WM_MOVE` refreshes the window) and on each change to the shown state;
//! - a meter frame: up to 30 a second while a room plays.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, TestDispatcher, Window, WindowBounds, WindowOptions, div,
    point, px, size,
};

use super::settings_window::SettingsView;
use super::{Live, LiveHandle, MainWindow, appearance};
use crate::controller::{AppState, Controller, EndpointView, ReceiverView, Reconnect};
use crate::level::LevelTap;
use crate::session::{LatencyProfile, Status};
use crate::settings::Settings;

/// A full frame of the main window with eight rooms playing, against the reference frame. It
/// was about 2.9 times the reference when this was set (2.3 to 2.5 with 16 copies of the test
/// running at once), and about 8.5 times before the layout fixes.
const FULL_FRAME_RATIO: f64 = 4.0;
/// The settings window against the reference frame: about 1.9 times when this was set. It has
/// no meters, but it is dragged too. The layout fixes took it from about 2.25 only, too close to
/// tell apart on every machine, so this limit catches a layout that grows by half or more:
/// seven nested stretched columns around its rows took it to 5.
const SETTINGS_FRAME_RATIO: f64 = 2.75;
/// A meter frame against a full frame of the same window: about 0.12 when this was set, and
/// about 1 if the content is drawn again for a meter.
const METER_FRAME_RATIO: f64 = 0.25;

/// Frames drawn before timing, so caches (text, images, keyed state) are warm.
const WARM_UP: usize = 5;
/// Pairs of frames timed; the median is used.
const SAMPLES: usize = 41;

/// Held by each timing test, so they do not time each other.
static TIMING: Mutex<()> = Mutex::new(());

fn room(i: usize, playing: bool) -> ReceiverView {
    ReceiverView {
        name: format!("Room {i}"),
        addr: format!("192.0.2.{}:7000", 10 + i).parse().unwrap(),
        model: Some("AudioAccessory5,1".into()),
        online: true,
        selected: playing,
        volume_pct: 50.0,
        volume_known: true,
        status: if playing {
            Status::Streaming
        } else {
            Status::Idle
        },
        stats: None,
        profile: LatencyProfile::Balanced,
        restarting: false,
        reconnect: Reconnect::No,
        suggestion: None,
        hidden: false,
        level: playing.then(|| Arc::new(LevelTap::new())),
    }
}

/// `rooms` found, the first `playing` of them on and playing.
fn state(rooms: usize, playing: usize) -> AppState {
    let endpoint = EndpointView {
        id: "demo-endpoint-0".into(),
        name: "Speakers (USB Audio DAC)".into(),
        is_default: true,
    };
    AppState {
        endpoints: vec![endpoint.clone()],
        endpoint: Some(endpoint),
        receivers: (0..rooms).map(|i| room(i, i < playing)).collect(),
        streaming: playing > 0,
        ..Default::default()
    }
}

/// A test app with the real text system.
fn app() -> TestAppContext {
    let text = gpui_kit::platform::current_platform(false).text_system();
    let cx = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text);
    cx.update(|cx| {
        assert!(
            !cx.text_system().all_font_names().is_empty(),
            "the frame tests need the real text system, which lists the installed fonts"
        );
        gpui_kit::init(cx);
        appearance::init(&Settings::default(), cx);
    });
    cx
}

fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(560.), px(760.)),
        ))),
        ..Default::default()
    }
}

/// A full frame of `window`: nothing reused, as on each step of a window drag.
fn refresh(cx: &TestAppContext, window: AnyWindowHandle) {
    cx.update(|cx| window.update(cx, |_, window, _| window.refresh()).unwrap());
}

/// A plain list of 300 one-line rows: the yardstick for this machine, build and load.
struct Reference;

impl Render for Reference {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .children((0..300).map(|i| div().h(px(20.)).child(format!("Reference row {i}"))))
    }
}

/// An app with the main window showing `state` and the reference window, each drawn once.
struct Harness {
    cx: TestAppContext,
    main: AnyWindowHandle,
    view: Entity<MainWindow>,
    live: Entity<Live>,
    reference: AnyWindowHandle,
}

impl Harness {
    fn open(state: AppState) -> Self {
        let cx = app();
        let ctl = Arc::new(Controller::detached(state.clone()));
        let (live, (main, view), reference) = cx.update(|cx| {
            let live = cx.new(|_| Live { ctl, state });
            cx.set_global(LiveHandle(live.clone()));
            let main = gpui_kit::open_window(options(), cx, |window, cx| {
                cx.new(|cx| MainWindow::new(live.clone(), window, cx))
            })
            .unwrap();
            let reference = gpui_kit::open_window(options(), cx, |_, cx| cx.new(|_| Reference))
                .unwrap()
                .0;
            (live, main, reference)
        });
        cx.run_until_parked();
        Self {
            cx,
            main,
            view,
            live,
            reference,
        }
    }

    /// Opens the settings window over the same state.
    fn open_settings(&self) -> AnyWindowHandle {
        let window = self.cx.update(|cx| {
            gpui_kit::open_window(options(), cx, |window, cx| {
                cx.new(|cx| SettingsView::new(self.live.clone(), window, cx))
            })
            .unwrap()
            .0
        });
        self.cx.run_until_parked();
        window
    }

    fn full_frame(&self) {
        refresh(&self.cx, self.main);
    }

    fn reference_frame(&self) {
        refresh(&self.cx, self.reference);
    }

    /// A frame for a meter that moved.
    fn meter_frame(&self) {
        let meters = self.cx.read(|cx| self.view.read(cx).meters.clone());
        self.cx.update(|cx| meters.update(cx, |_, cx| cx.notify()));
    }

    /// How many frames the title bar and the content have rendered in.
    fn renders(&self) -> (usize, usize) {
        self.cx.read(|cx| {
            let main = self.view.read(cx);
            (main.title.read(cx).renders, main.content.read(cx).renders)
        })
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.cx.quit();
    }
}

/// Frame times of `a` and `b`, timed in turn after a warm-up: the median of each, and the
/// median of the per-pair ratios `a / b`.
fn paired(mut a: impl FnMut(), mut b: impl FnMut()) -> (Duration, Duration, f64) {
    fn time(f: &mut impl FnMut()) -> Duration {
        let start = Instant::now();
        f();
        start.elapsed()
    }
    for _ in 0..WARM_UP {
        a();
        b();
    }
    let (mut ta, mut tb, mut ratios) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..SAMPLES {
        let (x, y) = (time(&mut a), time(&mut b));
        ratios.push(x.as_secs_f64() / y.as_secs_f64());
        ta.push(x);
        tb.push(y);
    }
    ta.sort();
    tb.sort();
    ratios.sort_by(f64::total_cmp);
    (ta[SAMPLES / 2], tb[SAMPLES / 2], ratios[SAMPLES / 2])
}

fn timing() -> MutexGuard<'static, ()> {
    TIMING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Windows asks the app on every click in the title bar whether it hit a caption button or
/// the drag area, and GPUI answers from the last frame. A part of the window replayed from
/// the cache carries no such areas (gpui-pre 0.3.9), so the title bar must render in every
/// frame, meter frames included, while the rooms are not drawn again for a meter.
#[test]
fn a_meter_frame_draws_the_title_bar_but_not_the_rooms() {
    let h = Harness::open(state(4, 4));
    let (title, content) = h.renders();
    h.meter_frame();
    assert_eq!(
        h.renders(),
        (title + 1, content),
        "a meter frame must draw the title bar and reuse the rooms"
    );
    h.full_frame();
    assert_eq!(h.renders(), (title + 2, content + 1));
}

#[test]
fn a_full_frame_stays_within_budget() {
    let _timing = timing();
    let h = Harness::open(state(8, 8));
    let (full, reference, ratio) = paired(|| h.full_frame(), || h.reference_frame());
    eprintln!("full frame, 8 rooms playing: {full:?} ({ratio:.2} x reference {reference:?})");
    assert!(
        ratio <= FULL_FRAME_RATIO,
        "a full frame took {ratio:.2} x the reference frame, over {FULL_FRAME_RATIO}"
    );
}

#[test]
fn a_meter_frame_costs_a_fraction_of_a_full_frame() {
    let _timing = timing();
    let h = Harness::open(state(8, 8));
    let (meter, full, ratio) = paired(|| h.meter_frame(), || h.full_frame());
    eprintln!("meter frame, 8 rooms playing: {meter:?} ({ratio:.2} x full frame {full:?})");
    assert!(
        ratio <= METER_FRAME_RATIO,
        "a meter frame took {ratio:.2} x a full frame, over {METER_FRAME_RATIO}"
    );
}

#[test]
fn a_full_frame_of_the_settings_stays_within_budget() {
    let _timing = timing();
    let h = Harness::open(state(8, 8));
    let settings = h.open_settings();
    let (full, reference, ratio) = paired(|| refresh(&h.cx, settings), || h.reference_frame());
    eprintln!("settings frame: {full:?} ({ratio:.2} x reference {reference:?})");
    assert!(
        ratio <= SETTINGS_FRAME_RATIO,
        "a settings frame took {ratio:.2} x the reference frame, over {SETTINGS_FRAME_RATIO}"
    );
}
