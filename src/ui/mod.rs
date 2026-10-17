//! gpui-kit windows: render [`AppState`] snapshots and send [`Command`]s to the controller.
//!
//! The controller thread calls the notifier after every state change; the notifier pushes a
//! wake-up into a `futures` unbounded channel, and a GPUI foreground task takes a fresh
//! snapshot into the [`Live`] entity, which both windows observe. Wake-ups are coalesced
//! with an `AtomicBool`.
//!
//! The look follows design/DESIGN.md: a pane of glass (acrylic) with the rooms on it and the
//! transport in a footer. `style` holds the tokens, `accent` the Windows accent, `appearance`
//! the look both windows share, `glass` the window material, `controls` the switch/slider/
//! icons, `rooms` the room rows and banners, `sync_note` the footer's note about rooms on
//! different profiles, `settings_window` the settings (opened from the gear in the title bar).

mod accent;
mod appearance;
mod controls;
mod glass;
mod rooms;
mod settings_window;
mod style;
mod sync_note;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc::unbounded;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::APP_TITLE;
use crate::controller::{AppState, Command, Controller, ReceiverView, Reconnect, SHUTDOWN_TIMEOUT};
use crate::session::{LatencyProfile, Status};
use crate::settings::Settings;

use appearance::{Appearance, WindowGlass};
use controls::icon;
use style::{Fonts, Palette, TypeExt as _, lead_text};

/// While the user drags a slider, controller snapshots do not move it (they may lag).
const USER_VOLUME_HOLD: Duration = Duration::from_millis(1500);
/// Minimum gap between live volume commands while dragging (the final value is always sent).
const VOLUME_SEND_INTERVAL: Duration = Duration::from_millis(80);
/// Below this width the room controls stack (DESIGN.md, "Narrow").
const NARROW_WIDTH: Pixels = px(440.);
/// The name in the title bar (the OS window title keeps "(alpha)").
const APP_NAME: &str = "Zephon";

/// The latest controller snapshot, shared by both windows.
pub struct Live {
    pub ctl: Arc<Controller>,
    pub state: AppState,
}

/// The [`Live`] entity, for windows opened later (settings).
struct LiveHandle(Entity<Live>);

impl Global for LiveHandle {}

#[derive(Clone)]
struct EndpointItem {
    id: String,
    name: SharedString,
    is_default: bool,
}

impl SearchableListItem for EndpointItem {
    type Value = String;
    fn title(&self) -> SharedString {
        self.name.clone()
    }
    fn value(&self) -> &String {
        &self.id
    }
    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .w_full()
            .gap_3()
            .justify_between()
            .child(div().min_w_0().truncate().child(self.name.clone()))
            .when(self.is_default, |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_size(Fonts::of(cx).px(12.))
                        .text_color(Palette::of(cx).ink_3)
                        .child("Windows default"),
                )
            })
    }
}

/// "Balanced" for the select; the lag figure follows in Ink 2.
fn profile_title(p: LatencyProfile) -> &'static str {
    match p {
        LatencyProfile::Low => "Low",
        LatencyProfile::Balanced => "Balanced",
        LatencyProfile::Stable => "Stable",
    }
}

#[derive(Clone)]
struct ProfileItem(LatencyProfile);

impl SearchableListItem for ProfileItem {
    type Value = LatencyProfile;
    fn title(&self) -> SharedString {
        format!("{} {} ms", profile_title(self.0), self.0.lag_ms()).into()
    }
    fn display_title(&self) -> Option<AnyElement> {
        Some(
            ProfileLabel {
                profile: self.0,
                menu: false,
            }
            .into_any_element(),
        )
    }
    fn value(&self) -> &LatencyProfile {
        &self.0
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        ProfileLabel {
            profile: self.0,
            menu: true,
        }
    }
}

/// "Balanced 165 ms" with the figure in Ink 2; the menu adds "lag behind this PC".
#[derive(IntoElement)]
struct ProfileLabel {
    profile: LatencyProfile,
    menu: bool,
}

impl RenderOnce for ProfileLabel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Palette::of(cx);
        let label = h_flex().gap_1p5().child(profile_title(self.profile)).child(
            div()
                .text_color(p.ink_2)
                .child(format!("{} ms", self.profile.lag_ms())),
        );
        if !self.menu {
            return label;
        }
        h_flex()
            .w_full()
            .gap_3()
            .justify_between()
            .child(label)
            .child(
                div()
                    .text_size(Fonts::of(cx).px(12.))
                    .text_color(p.ink_3)
                    .child("lag behind this PC"),
            )
    }
}

/// Stateful widgets of one room row.
struct RowWidgets {
    slider: Entity<SliderState>,
    /// Slider value shown (0-100).
    value: f32,
    last_user: Option<Instant>,
    last_sent: Option<Instant>,
    profile: Entity<SelectState<Vec<ProfileItem>>>,
}

struct SenderView {
    ctl: Arc<Controller>,
    live: Entity<Live>,
    /// The snapshot shown, without hidden rooms.
    state: AppState,
    endpoint: Entity<SelectState<Vec<EndpointItem>>>,
    /// (id, name, is_default) of the items currently in the endpoint select.
    endpoint_items: Vec<(String, String, bool)>,
    rows: HashMap<String, RowWidgets>,
    fonts: Fonts,
    glass: WindowGlass,
    subscriptions: Vec<Subscription>,
}

impl SenderView {
    fn new(live: Entity<Live>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let endpoint = cx.new(|cx| SelectState::new(Vec::<EndpointItem>::new(), None, window, cx));

        let subscriptions = vec![
            cx.subscribe(
                &endpoint,
                |this, _, ev: &SelectEvent<Vec<EndpointItem>>, _| {
                    let SelectEvent::Confirm(Some(id)) = ev else {
                        return;
                    };
                    this.ctl.send(Command::SelectEndpoint(id.clone()));
                },
            ),
            cx.observe_in(&live, window, |this, _, window, cx| {
                this.refresh(window, cx)
            }),
            // Theme, material, font, text size or accent changed (here or in the settings).
            cx.observe_global_in::<Appearance>(window, |this, window, cx| {
                this.fonts = Fonts::of(cx);
                this.glass.sync(window, cx);
                cx.notify();
            }),
            // "System" follows Windows' light/dark setting live.
            cx.observe_window_appearance(window, |this, window, cx| {
                this.glass.on_os_appearance(window, cx)
            }),
        ];

        let mut view = Self {
            ctl: live.read(cx).ctl.clone(),
            live,
            state: AppState::default(),
            endpoint,
            endpoint_items: Vec::new(),
            rows: HashMap::new(),
            fonts: Fonts::of(cx),
            glass: WindowGlass::new(),
            subscriptions,
        };
        view.glass.sync(window, cx);
        view.refresh(window, cx);
        view
    }

    /// Takes the latest snapshot and syncs the stateful widgets (selects, sliders) to it.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state = self.live.read(cx).state.clone();
        // Hidden rooms are left out everywhere in this window.
        self.state.receivers.retain(|r| !r.hidden);

        // Capture endpoint select.
        let items: Vec<(String, String, bool)> = self
            .state
            .endpoints
            .iter()
            .map(|e| (e.id.clone(), e.name.clone(), e.is_default))
            .collect();
        let chosen = self.state.endpoint.as_ref().map(|e| e.id.clone());
        let items_changed = items != self.endpoint_items;
        if items_changed {
            self.endpoint_items = items.clone();
        }
        self.endpoint.update(cx, |sel, cx| {
            if items_changed {
                let list = items
                    .into_iter()
                    .map(|(id, name, is_default)| EndpointItem {
                        id,
                        name: name.into(),
                        is_default,
                    })
                    .collect();
                sel.set_items(list, window, cx);
            }
            if items_changed || sel.selected_value() != chosen.as_ref() {
                match &chosen {
                    Some(id) => sel.set_selected_value(id, window, cx),
                    None => sel.set_selected_index(None, window, cx),
                }
            }
        });

        // One profile select and one volume slider per room; receiver-reported volume
        // moves the slider.
        let receivers = self.state.receivers.clone();
        for r in &receivers {
            if !self.rows.contains_key(&r.name) {
                self.add_row(r, window, cx);
            }
            let Some(row) = self.rows.get_mut(&r.name) else {
                continue;
            };
            let user_active = row
                .last_user
                .is_some_and(|t| t.elapsed() < USER_VOLUME_HOLD);
            if !user_active && (row.value - r.volume_pct).abs() >= 0.5 {
                row.value = r.volume_pct;
                row.slider
                    .update(cx, |s, cx| s.set_value(r.volume_pct, window, cx));
            }
            row.profile.update(cx, |sel, cx| {
                if sel.selected_value() != Some(&r.profile) {
                    sel.set_selected_value(&r.profile, window, cx);
                }
            });
        }
        cx.notify();
    }

    fn add_row(&mut self, r: &ReceiverView, window: &mut Window, cx: &mut Context<Self>) {
        let profiles: Vec<ProfileItem> = LatencyProfile::ALL
            .iter()
            .map(|p| ProfileItem(*p))
            .collect();
        // `refresh` selects the receiver's profile right after this.
        let profile = cx.new(|cx| SelectState::new(profiles, None, window, cx));
        let name = r.name.clone();
        self.subscriptions.push(cx.subscribe(
            &profile,
            move |this, _, ev: &SelectEvent<Vec<ProfileItem>>, _| {
                let SelectEvent::Confirm(Some(p)) = ev else {
                    return;
                };
                this.ctl.send(Command::SetProfile(name.clone(), *p));
            },
        ));

        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(r.volume_pct.round())
        });
        let name = r.name.clone();
        self.subscriptions
            .push(cx.subscribe(&slider, move |this, _, ev: &SliderEvent, cx| {
                let (value, last) = match ev {
                    SliderEvent::Change(v) => (v.start(), false),
                    SliderEvent::Release(v) => (v.start(), true),
                };
                let Some(row) = this.rows.get_mut(&name) else {
                    return;
                };
                let now = Instant::now();
                row.value = value;
                row.last_user = Some(now);
                if last
                    || row
                        .last_sent
                        .is_none_or(|t| t.elapsed() >= VOLUME_SEND_INTERVAL)
                {
                    row.last_sent = Some(now);
                    this.ctl.send(Command::SetVolume(name.clone(), value));
                }
                cx.notify();
            }));
        self.rows.insert(
            r.name.clone(),
            RowWidgets {
                slider,
                value: r.volume_pct.round(),
                last_user: None,
                last_sent: None,
                profile,
            },
        );
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::of(cx);
        title_bar(APP_NAME, &self.fonts, &p).child(
            // Occlude, so clicks reach the button instead of the window-drag area.
            h_flex()
                .id("title-actions")
                .occlude()
                .mr_1()
                .items_center()
                .child(
                    // Medium icon buttons are 32 px square with a 16 px icon.
                    Button::new("settings")
                        .ghost()
                        .icon(icon(controls::GEAR, 16., p.ink_2))
                        .tooltip("Settings")
                        .accessibility_label("Settings")
                        .on_click(|_, window, cx| settings_window::open(window, cx)),
                ),
        )
    }

    fn render_sound_from(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::of(cx);
        let streaming = self.state.streaming;
        let fonts = &self.fonts;
        let source: AnyElement = if streaming {
            // Locked while playing: plain text in an outline, no chevron.
            div()
                .h(fonts.px(32.))
                .px(px(11.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(p.stroke)
                .t_body(fonts)
                .text_color(p.ink)
                .child(
                    div().min_w_0().truncate().child(
                        self.state
                            .endpoint
                            .as_ref()
                            .map(|e| e.name.clone())
                            .unwrap_or_default(),
                    ),
                )
                .into_any_element()
        } else {
            // The device list is re-read each time the select is opened.
            div()
                .id("sound-from")
                .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                    this.ctl.send(Command::RefreshEndpoints);
                }))
                .child(
                    Select::new(&self.endpoint)
                        .placeholder("Choose where the sound comes from")
                        .accessibility_label("Sound from")
                        .bg(p.control_on_root)
                        .border_color(p.stroke),
                )
                .into_any_element()
        };
        v_flex()
            .child(
                h_flex()
                    .justify_between()
                    .mb(px(6.))
                    .t_small(fonts)
                    .text_color(p.ink_2)
                    .child("Sound from")
                    .when(streaming, |this| this.child("Stop to change")),
            )
            .child(source)
            .children(
                self.state
                    .endpoint_error
                    .clone()
                    .map(|e| div().mt(px(6.)).t_small(fonts).text_color(p.red).child(e)),
            )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::of(cx);
        let fonts = &self.fonts;
        let st = &self.state;
        let streaming = st.streaming;
        let on = st.receivers.iter().filter(|r| r.selected).count();
        let playing = st
            .receivers
            .iter()
            .filter(|r| r.selected && r.status == Status::Streaming && !r.restarting)
            .count();
        let rooms = |n: usize| {
            if n == 1 {
                "1 room".to_string()
            } else {
                format!("{n} rooms")
            }
        };
        let reconnecting = st
            .receivers
            .iter()
            .any(|r| r.selected && matches!(r.reconnect, Reconnect::Attempt(_)));
        // Once no room that is on is left playing or trying, the controller stops, so the
        // button turns back into Play.
        let summary = if streaming {
            if playing > 0 {
                lead_text("Playing", &format!("in {}", rooms(playing)), p.ink)
            } else if reconnecting {
                lead_text("Reconnecting…", "", p.ink)
            } else if on == 0 {
                StyledText::new("No rooms on")
            } else {
                lead_text("Connecting…", "", p.ink)
            }
        } else if on == 0 {
            StyledText::new("No rooms on")
        } else {
            StyledText::new(format!("{} on", rooms(on)))
        };
        let transport = if streaming {
            Button::new("transport")
                .icon(icon(controls::STOP, 14., p.ink))
                .label("Stop")
        } else {
            Button::new("transport")
                .primary()
                .icon(icon(controls::PLAY, 14., p.on_lagoon))
                .label("Play")
        };

        v_flex()
            .flex_none()
            .bg(p.group_off)
            .border_t_1()
            .border_color(p.stroke)
            .pt(px(14.))
            .px(px(16.))
            .pb(px(16.))
            .children(sync_note::sync_note(&st.receivers, streaming).map(|n| {
                div()
                    .mb(px(10.))
                    .t_small(fonts)
                    .text_color(p.ink_2)
                    .child(n)
            }))
            .children(st.message.clone().map(|m| {
                div()
                    .mb(px(10.))
                    .t_small(fonts)
                    .text_color(p.amber)
                    .child(m)
            }))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .t_body(fonts)
                            .text_color(p.ink_2)
                            .child(summary),
                    )
                    .child(
                        transport
                            .min_w(fonts.px(112.))
                            .px(px(14.))
                            .gap_2()
                            .on_click(cx.listener(|this, _, _, _| {
                                let cmd = if this.state.streaming {
                                    Command::Stop
                                } else {
                                    Command::Start
                                };
                                this.ctl.send(cmd);
                            })),
                    ),
            )
    }
}

impl Render for SenderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::of(cx);
        let narrow = window.viewport_size().width < NARROW_WIDTH * self.fonts.scale;

        // Every pixel's tint is painted exactly once (see `style`): the root tint on each
        // strip around the rooms group, the rows and the footer their own precomposited tint.
        let strip = || div().flex_none().bg(p.root);
        let (rooms_header, rooms_group) = rooms::render_rooms(self, narrow, window, cx);
        v_flex()
            .size_full()
            .t_body(&self.fonts)
            .text_color(p.ink)
            .child(self.render_title_bar(cx))
            .child(
                v_flex()
                    .id("content")
                    .flex_1()
                    .min_h_0()
                    .child(
                        strip()
                            .px(px(16.))
                            .pt(px(2.))
                            .pb(px(18.))
                            .child(self.render_sound_from(cx)),
                    )
                    .children(
                        rooms::render_firewall(self, cx)
                            .map(|banner| strip().px(px(16.)).pb(px(18.)).child(banner)),
                    )
                    .child(strip().px(px(16.)).pb(px(6.)).child(rooms_header))
                    .child(controls::group_on_glass(rooms_group, &p))
                    // The rest of the pane, down to the footer.
                    .child(div().flex_1().min_h(px(16.)).bg(p.root))
                    .overflow_y_scrollbar(),
            )
            .child(self.render_footer(cx))
    }
}

/// The title bar both windows use: the glyph and a name on the glass. Add the actions as a
/// child.
fn title_bar(name: &'static str, fonts: &Fonts, p: &Palette) -> TitleBar {
    TitleBar::new()
        .h(px(40.))
        .bg(p.root)
        .border_color(gpui_kit::transparent_black())
        .pl(px(14.))
        .child(
            h_flex()
                .gap(px(10.))
                .items_center()
                .child(icon(controls::GLYPH, 16., p.lagoon))
                .child(div().t_small(fonts).text_color(p.ink).child(name)),
        )
}

/// Window options for a window with our title bar on glass.
fn window_options(title: &str, bounds: Bounds<Pixels>, min: Size<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(min),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_string().into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    }
}

/// Opens the window and runs until it is closed. Sessions are torn down on close.
pub fn run() {
    // Read before the controller starts, so a settings migration cannot race it.
    let saved = Settings::load();

    let (tx, mut rx) = unbounded::<()>();
    let pending = Arc::new(AtomicBool::new(false));
    let notify_pending = pending.clone();
    let ctl = Arc::new(Controller::spawn(Arc::new(move || {
        if !notify_pending.swap(true, Ordering::SeqCst) {
            let _ = tx.unbounded_send(());
        }
    })));
    let on_exit = ctl.clone();

    application().with_assets(assets::Assets).run(move |cx| {
        // gpui-kit's init loads its own light theme; ours replace it right after, with the
        // saved theme, accent, font and text size.
        init(cx);
        appearance::init(&saved, cx);

        // Controller wake-ups refresh the shared snapshot; both windows observe it.
        let live = cx.new(|_| Live {
            ctl: ctl.clone(),
            state: ctl.snapshot(),
        });
        cx.set_global(LiveHandle(live.clone()));
        let updates = live.clone();
        cx.spawn(async move |cx| {
            while rx.next().await.is_some() {
                updates.update(cx, |live, cx| {
                    pending.store(false, Ordering::SeqCst);
                    live.state = live.ctl.snapshot();
                    cx.notify();
                });
            }
        })
        .detach();

        // A new Windows accent applies live (the settings app writes several values; wait
        // for the last).
        let (accent_tx, mut accent_rx) = unbounded::<()>();
        accent::watch(move || accent_tx.unbounded_send(()).is_ok());
        cx.spawn(async move |cx| {
            while accent_rx.next().await.is_some() {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                while accent_rx.try_recv().is_ok() {}
                cx.update(appearance::refresh_windows_accent);
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(560.), px(760.)), cx);
        let options = window_options(APP_TITLE, bounds, size(px(360.), px(420.)));
        match open_window(options, cx, move |window, cx| {
            glass::init_frame(window);
            cx.new(|cx| SenderView::new(live, window, cx))
        }) {
            Ok((main, _)) => {
                // Closing the main window quits (closing the settings, too); closing the
                // settings window only forgets it. The sessions are torn down after the
                // event loop has ended (below), so no window sits frozen meanwhile.
                cx.on_window_closed(move |cx, id| {
                    if id == main.window_id() {
                        for w in cx.windows() {
                            let _ = w.update(cx, |_, window, _| window.remove_window());
                        }
                        cx.quit();
                    } else {
                        settings_window::closed(id, cx);
                    }
                })
                .detach();
                ctl.send(Command::Resume);
            }
            Err(e) => {
                tracing::error!("Could not open the window: {e:#}");
                cx.quit();
            }
        }
    });
    // The windows are gone: TEARDOWN every session (bounded) before the process exits.
    on_exit.shutdown(SHUTDOWN_TIMEOUT);
}
