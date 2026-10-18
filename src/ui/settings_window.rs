//! The settings window (DESIGN.md, "Settings window"), opened from the gear in the main
//! title bar. One at a time: the gear focuses it when it is open.
//!
//! Windows 11 Settings layout on the same glass as the main window: sections (Appearance,
//! Playback, Rooms, System, About), each a rounded group of rows; a row is a label with a
//! one-line description in Ink 2 on the left and its control on the right. Every change is
//! sent to the controller, which saves it at once; the look (theme, material, font, text
//! size, accent) also goes through `appearance`, so both windows change together.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::appearance::{self, Appearance, WindowGlass};
use super::controls::{self, group_on_glass};
use super::rooms::model_name;
use super::style::{DEFAULT_FONT_NAME, Fonts, Palette, TypeExt as _};
use super::{Live, LiveHandle, ProfileItem, glass};
use crate::autostart::{self, Autostart};
use crate::controller::{AppState, Command, Controller, known_rooms};
use crate::firewall::FirewallState;
use crate::session::{LatencyProfile, Status};
use crate::settings::{self, AccentChoice, Backdrop, TextSize, ThemeChoice};

const WINDOW_TITLE: &str = "Zephon settings";
/// Below this width (times the text size) a row's control moves under its label.
const NARROW_WIDTH: f32 = 440.;
/// Width of the selects and the volume slider on the right of a row.
const CONTROL_WIDTH: f32 = 180.;
/// See the main window: while dragging, snapshots do not move the slider.
const USER_VOLUME_HOLD: Duration = Duration::from_millis(1500);
/// Minimum gap between saves of the new-room volume while dragging.
const VOLUME_SAVE_INTERVAL: Duration = Duration::from_millis(250);

/// The open settings window.
struct OpenSettings(AnyWindowHandle);

impl Global for OpenSettings {}

/// Opens the settings window over `parent` (the main window), or brings it to the front when
/// it is open.
pub fn open(parent: &Window, cx: &mut App) {
    if let Some(handle) = cx.try_global::<OpenSettings>().map(|o| o.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let Some(live) = cx.try_global::<LiveHandle>().map(|l| l.0.clone()) else {
        return;
    };
    // Centred over the main window, kept inside the display under its centre.
    let center = parent.bounds().center();
    let mut bounds = Bounds::centered_at(center, size(px(520.), px(640.)));
    let display = cx
        .displays()
        .into_iter()
        .find(|d| d.bounds().contains(&center))
        .or_else(|| parent.display(cx));
    if let Some(area) = display.map(|d| d.visible_bounds()) {
        let fit = |v: Pixels, lo: Pixels, hi: Pixels| {
            if v > hi {
                hi
            } else if v < lo {
                lo
            } else {
                v
            }
        };
        bounds.size = bounds.size.min(&area.size);
        let end = area.origin + point(area.size.width, area.size.height);
        bounds.origin.x = fit(bounds.origin.x, area.origin.x, end.x - bounds.size.width);
        bounds.origin.y = fit(bounds.origin.y, area.origin.y, end.y - bounds.size.height);
    }
    let options = super::window_options(WINDOW_TITLE, bounds, size(px(380.), px(360.)));
    match open_window(options, cx, move |window, cx| {
        glass::init_frame(window);
        cx.new(|cx| SettingsView::new(live, window, cx))
    }) {
        Ok((handle, _)) => cx.set_global(OpenSettings(handle)),
        Err(e) => tracing::error!("Could not open the settings window: {e:#}"),
    }
}

/// Forgets the settings window once it is closed.
pub fn closed(id: WindowId, cx: &mut App) {
    if cx
        .try_global::<OpenSettings>()
        .is_some_and(|o| o.0.window_id() == id)
    {
        cx.remove_global::<OpenSettings>();
    }
}

/// One option of a fixed select.
#[derive(Clone)]
struct Choice<T: Clone + PartialEq + 'static> {
    value: T,
    title: &'static str,
}

impl<T: Clone + PartialEq + 'static> SearchableListItem for Choice<T> {
    type Value = T;
    fn title(&self) -> SharedString {
        self.title.into()
    }
    fn value(&self) -> &T {
        &self.value
    }
}

fn choices<T: Clone + PartialEq + 'static>(list: &[(T, &'static str)]) -> Vec<Choice<T>> {
    list.iter()
        .map(|(value, title)| Choice {
            value: value.clone(),
            title,
        })
        .collect()
}

/// A font family; `None` is the default (Segoe UI Variable's optical sizes).
#[derive(Clone)]
struct FontItem {
    family: Option<String>,
    title: SharedString,
}

impl SearchableListItem for FontItem {
    type Value = Option<String>;
    fn title(&self) -> SharedString {
        self.title.clone()
    }
    fn value(&self) -> &Option<String> {
        &self.family
    }
    /// Each family in its own face; the default in the UI face, marked as the default.
    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let fonts = Fonts::of(cx);
        h_flex()
            .w_full()
            .gap_3()
            .justify_between()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .when_some(self.family.clone(), |this, f| this.font_family(f))
                    .child(self.title.clone()),
            )
            .when(self.family.is_none(), |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_size(fonts.px(12.))
                        .text_color(Palette::of(cx).ink_3)
                        .child("Default"),
                )
            })
    }
}

fn material_name(b: Backdrop) -> &'static str {
    match b {
        Backdrop::Acrylic => "Acrylic",
        Backdrop::Mica => "Mica",
        Backdrop::Solid => "Solid",
    }
}

type ChoiceSelect<T> = Entity<SelectState<Vec<Choice<T>>>>;

struct SettingsView {
    ctl: Arc<Controller>,
    live: Entity<Live>,
    fonts: Fonts,
    glass: WindowGlass,
    theme: ChoiceSelect<ThemeChoice>,
    material: ChoiceSelect<Backdrop>,
    // SearchableVec, not Vec: a plain Vec delegate ignores the search query.
    font: Entity<SelectState<SearchableVec<FontItem>>>,
    text_size: ChoiceSelect<TextSize>,
    accent: ChoiceSelect<AccentChoice>,
    profile: Entity<SelectState<Vec<ProfileItem>>>,
    volume: Entity<SliderState>,
    /// New-room volume shown (0-100).
    volume_value: f32,
    volume_last_user: Option<Instant>,
    volume_last_sent: Option<Instant>,
    /// The Run value, read when the window opens and after each change.
    autostart: Autostart,
    autostart_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    fn new(live: Entity<Live>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let a = cx.global::<Appearance>();
        let (theme_now, material_now, text_size_now, accent_now) =
            (a.theme, a.backdrop, a.text_size, a.accent);
        let mut font_items = vec![FontItem {
            family: None,
            title: DEFAULT_FONT_NAME.into(),
        }];
        font_items.extend(a.font_names.iter().map(|f| FontItem {
            family: Some(f.clone()),
            title: f.clone().into(),
        }));
        let font_now = if a.font_missing { None } else { a.font.clone() };
        let saved = live.read(cx).state.settings.clone();

        let theme = cx.new(|cx| {
            let list = choices(&[
                (ThemeChoice::System, "System"),
                (ThemeChoice::Light, "Light"),
                (ThemeChoice::Dark, "Dark"),
            ]);
            let mut s = SelectState::new(list, None, window, cx);
            s.set_selected_value(&theme_now, window, cx);
            s
        });
        let material = cx.new(|cx| {
            let list = choices(&[
                (Backdrop::Acrylic, "Acrylic"),
                (Backdrop::Mica, "Mica"),
                (Backdrop::Solid, "Solid"),
            ]);
            let mut s = SelectState::new(list, None, window, cx);
            s.set_selected_value(&material_now, window, cx);
            s
        });
        let font = cx.new(|cx| {
            let items = SearchableVec::new(font_items);
            let mut s = SelectState::new(items, None, window, cx).searchable(true);
            s.set_selected_value(&font_now, window, cx);
            s
        });
        let text_size = cx.new(|cx| {
            let list = choices(&[
                (TextSize::Small, "Small"),
                (TextSize::Default, "Default"),
                (TextSize::Large, "Large"),
            ]);
            let mut s = SelectState::new(list, None, window, cx);
            s.set_selected_value(&text_size_now, window, cx);
            s
        });
        let accent = cx.new(|cx| {
            let list = choices(&[
                (AccentChoice::Lagoon, "Lagoon"),
                (AccentChoice::Windows, "Windows accent"),
            ]);
            let mut s = SelectState::new(list, None, window, cx);
            s.set_selected_value(&accent_now, window, cx);
            s
        });
        let profile = cx.new(|cx| {
            let list: Vec<ProfileItem> = LatencyProfile::ALL
                .iter()
                .map(|p| ProfileItem(*p))
                .collect();
            let mut s = SelectState::new(list, None, window, cx);
            s.set_selected_value(&saved.default_profile, window, cx);
            s
        });
        let volume_value = saved.default_volume.round();
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(volume_value)
        });

        let subscriptions = vec![
            cx.subscribe(
                &theme,
                |this, _, ev: &SelectEvent<Vec<Choice<ThemeChoice>>>, cx| {
                    let SelectEvent::Confirm(Some(t)) = ev else {
                        return;
                    };
                    let t = *t;
                    this.ctl.send(Command::SetTheme(t));
                    appearance::update(cx, |a| a.theme = t);
                },
            ),
            cx.subscribe(
                &material,
                |this, _, ev: &SelectEvent<Vec<Choice<Backdrop>>>, cx| {
                    let SelectEvent::Confirm(Some(b)) = ev else {
                        return;
                    };
                    let b = *b;
                    this.ctl.send(Command::SetBackdrop(b));
                    appearance::update(cx, |a| a.backdrop = b);
                },
            ),
            cx.subscribe(
                &font,
                |this, _, ev: &SelectEvent<SearchableVec<FontItem>>, cx| {
                    let SelectEvent::Confirm(Some(f)) = ev else {
                        return;
                    };
                    let f = f.clone();
                    this.ctl.send(Command::SetFont(f.clone()));
                    appearance::update(cx, |a| a.font = f);
                },
            ),
            cx.subscribe(
                &text_size,
                |this, _, ev: &SelectEvent<Vec<Choice<TextSize>>>, cx| {
                    let SelectEvent::Confirm(Some(s)) = ev else {
                        return;
                    };
                    let s = *s;
                    this.ctl.send(Command::SetTextSize(s));
                    appearance::update(cx, |a| a.text_size = s);
                },
            ),
            cx.subscribe(
                &accent,
                |this, _, ev: &SelectEvent<Vec<Choice<AccentChoice>>>, cx| {
                    let SelectEvent::Confirm(Some(c)) = ev else {
                        return;
                    };
                    let c = *c;
                    this.ctl.send(Command::SetAccent(c));
                    appearance::update(cx, |a| a.accent = c);
                },
            ),
            cx.subscribe(
                &profile,
                |this, _, ev: &SelectEvent<Vec<ProfileItem>>, _| {
                    let SelectEvent::Confirm(Some(p)) = ev else {
                        return;
                    };
                    this.ctl.send(Command::SetDefaultProfile(*p));
                },
            ),
            cx.subscribe(&volume, |this, _, ev: &SliderEvent, cx| {
                let (value, last) = match ev {
                    SliderEvent::Change(v) => (v.start(), false),
                    SliderEvent::Release(v) => (v.start(), true),
                };
                let now = Instant::now();
                this.volume_value = value;
                this.volume_last_user = Some(now);
                // Each one is saved, so a drag sends a few (the final value always).
                if last
                    || this
                        .volume_last_sent
                        .is_none_or(|t| t.elapsed() >= VOLUME_SAVE_INTERVAL)
                {
                    this.volume_last_sent = Some(now);
                    this.ctl.send(Command::SetDefaultVolume(value));
                }
                cx.notify();
            }),
            cx.observe_in(&live, window, |this, _, window, cx| this.sync(window, cx)),
            cx.observe_global_in::<Appearance>(window, |this, window, cx| {
                this.fonts = Fonts::of(cx);
                this.glass.sync(window, cx);
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                this.glass.on_os_appearance(window, cx)
            }),
        ];

        let mut view = Self {
            ctl: live.read(cx).ctl.clone(),
            live,
            fonts: Fonts::of(cx),
            glass: WindowGlass::new(),
            theme,
            material,
            font,
            text_size,
            accent,
            profile,
            volume,
            volume_value,
            volume_last_user: None,
            volume_last_sent: None,
            autostart: autostart::state(),
            autostart_error: None,
            _subscriptions: subscriptions,
        };
        view.glass.sync(window, cx);
        view
    }

    /// Follows the saved new-room defaults (the other values are read when rendering).
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = &self.live.read(cx).state.settings;
        let (profile, volume) = (saved.default_profile, saved.default_volume.round());
        self.profile.update(cx, |sel, cx| {
            if sel.selected_value() != Some(&profile) {
                sel.set_selected_value(&profile, window, cx);
            }
        });
        let user_active = self
            .volume_last_user
            .is_some_and(|t| t.elapsed() < USER_VOLUME_HOLD);
        if !user_active && (self.volume_value - volume).abs() >= 0.5 {
            self.volume_value = volume;
            self.volume
                .update(cx, |s, cx| s.set_value(volume, window, cx));
        }
        cx.notify();
    }

    fn set_autostart(&mut self, on: bool, cx: &mut Context<Self>) {
        self.autostart_error = autostart::set(on).err();
        self.autostart = autostart::state();
        cx.notify();
    }

    fn select<T: Clone + PartialEq + 'static>(
        &self,
        state: &Entity<SelectState<Vec<Choice<T>>>>,
        label: &'static str,
        p: &Palette,
    ) -> AnyElement {
        Select::new(state)
            .w_full()
            .accessibility_label(label)
            .bg(p.control)
            .border_color(p.stroke)
            .into_any_element()
    }

    fn appearance_rows(&self, p: &Palette, cx: &App) -> Vec<Div> {
        let fonts = &self.fonts;
        let a = cx.global::<Appearance>();
        let material_desc = if self.glass.in_use == a.backdrop {
            Desc::plain("Acrylic blurs what's behind; Mica tints.")
        } else {
            Desc::warn(format!(
                "{} isn't available here; using {}.",
                material_name(a.backdrop),
                material_name(self.glass.in_use)
            ))
        };
        let font_desc = match (&a.font, a.font_missing) {
            (Some(f), true) => Desc::warn(format!(
                "\u{201C}{f}\u{201D} isn't installed; using the default."
            )),
            _ => Desc::plain("Used in both windows."),
        };
        let accent_desc = match (a.accent, a.windows_accent) {
            (AccentChoice::Windows, None) => Desc::warn("Couldn't read it; using Lagoon."),
            (AccentChoice::Windows, Some(_)) => {
                Desc::plain("Adjusted so text on it stays readable.")
            }
            _ => Desc::plain("Lagoon, or your Windows accent color."),
        };
        let font_select = Select::new(&self.font)
            .w_full()
            .accessibility_label("Font")
            .search_placeholder("Search fonts")
            .menu_width(fonts.px(260.))
            .bg(p.control)
            .border_color(p.stroke)
            .into_any_element();
        vec![
            row(
                "Theme",
                Desc::plain("System follows the Windows setting."),
                self.select(&self.theme, "Theme", p),
            ),
            row(
                "Window material",
                material_desc,
                self.select(&self.material, "Window material", p),
            ),
            row("Font", font_desc, font_select),
            row(
                "Text size",
                Desc::plain("Scales the text in both windows."),
                self.select(&self.text_size, "Text size", p),
            ),
            row(
                "Accent color",
                accent_desc,
                self.select(&self.accent, "Accent color", p),
            ),
        ]
    }

    fn playback_rows(
        &self,
        st: &AppState,
        p: &Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let fonts = &self.fonts;
        let s = &st.settings;
        let ctl = self.ctl.clone();
        let resume = controls::switch(
            "resume-on-launch",
            s.resume_on_launch,
            "Resume on launch".into(),
            move |on, _, _| ctl.send(Command::SetResumeOnLaunch(on)),
            window,
            cx,
        )
        .into_any_element();
        let ctl = self.ctl.clone();
        let follow = controls::switch(
            "volume-follows",
            s.volume_follows_receiver,
            "Volume follows phone".into(),
            move |on, _, _| ctl.send(Command::SetVolumeFollowsReceiver(on)),
            window,
            cx,
        )
        .into_any_element();
        let profile = Select::new(&self.profile)
            .w_full()
            .menu_width(fonts.px(260.))
            .accessibility_label("New-room latency profile")
            .bg(p.control)
            .border_color(p.stroke)
            .into_any_element();
        let volume = h_flex()
            .w_full()
            .gap(px(10.))
            .items_center()
            .child(controls::volume_slider(&self.volume, None, cx))
            .child(
                div()
                    .w(fonts.px(26.))
                    .flex_none()
                    .text_right()
                    .child(format!("{:.0}", self.volume_value)),
            )
            .into_any_element();
        vec![
            row(
                "New-room latency",
                Desc::plain("For rooms switched on the first time."),
                profile,
            ),
            row(
                "New-room volume",
                Desc::plain("Their start volume."),
                volume,
            ),
            row(
                "Resume on launch",
                Desc::plain("If it was playing at close, plays again."),
                resume,
            ),
            row(
                "Volume follows phone",
                Desc::plain("Phone or HomePod changes move the slider."),
                follow,
            ),
        ]
    }

    fn room_rows(
        &self,
        st: &AppState,
        p: &Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        known_rooms(&st.receivers, &st.settings)
            .into_iter()
            .map(|room| {
                let playing = st.streaming
                    && st.receivers.iter().any(|r| {
                        r.name == room.name
                            && r.selected
                            && matches!(r.status, Status::Streaming | Status::Connecting)
                    });
                let desc = if playing {
                    Desc::plain("Playing now. Hiding it stops it.")
                } else {
                    Desc::plain(match (room.online, room.model.as_deref().map(model_name)) {
                        (true, Some(model)) => model.to_string(),
                        (true, None) => "On the network".to_string(),
                        (false, Some(model)) => format!("{model}, offline"),
                        (false, None) => "Not found now".to_string(),
                    })
                };
                let name = room.name.clone();
                let ctl = self.ctl.clone();
                let show = controls::switch(
                    ElementId::Name(format!("show-{}", room.name).into()),
                    !room.hidden,
                    format!("Show {}", room.name).into(),
                    move |on, _, _| ctl.send(Command::SetHidden(name.clone(), !on)),
                    window,
                    cx,
                );
                row(room.name, desc, show.into_any_element())
                    .when(room.hidden, |this| this.text_color(p.ink_2))
            })
            .collect()
    }

    fn system_rows(&self, st: &AppState, window: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
        let autostart_desc = match (&self.autostart_error, &self.autostart) {
            (Some(e), _) => Desc::warn(format!("Couldn't change it: {e}")),
            (None, Autostart::OtherCopy(path)) => Desc::warn(format!(
                "Starts another copy: {path}. Switch off and on for this one."
            )),
            _ => Desc::plain("Opens at sign-in. See Resume on launch."),
        };
        let this = cx.entity().downgrade();
        let start = controls::switch(
            "start-with-windows",
            self.autostart != Autostart::Off,
            "Start with Windows".into(),
            move |on, _, cx| {
                let _ = this.update(cx, |view, cx| view.set_autostart(on, cx));
            },
            window,
            cx,
        )
        .into_any_element();

        let fw = &st.firewall;
        let firewall_desc = match fw {
            FirewallState::Ok => Desc::plain("Inbound TCP and UDP are allowed."),
            FirewallState::Unknown | FirewallState::Checking => {
                Desc::plain("Checking the inbound rules…")
            }
            FirewallState::Installing => Desc::plain("Windows asks for admin permission once."),
            FirewallState::Missing { blocked: true, .. } => Desc::warn(
                "A firewall rule blocks this app, so rooms can't send their timing replies.",
            ),
            FirewallState::Missing {
                tcp: true,
                udp: false,
                ..
            } => Desc::warn("Only TCP is allowed. Rooms send their timing replies over UDP."),
            FirewallState::Missing {
                tcp: false,
                udp: true,
                ..
            } => Desc::warn("Only UDP is allowed. Rooms also need TCP."),
            FirewallState::Missing { .. } => Desc::warn(
                "No inbound rule for this app, so rooms can't send their timing replies.",
            ),
            FirewallState::Cancelled => {
                Desc::warn("Windows didn't get permission, so nothing changed.")
            }
            FirewallState::Error(e) => Desc::warn(e.clone()),
        };
        let busy = matches!(
            fw,
            FirewallState::Unknown | FirewallState::Checking | FirewallState::Installing
        );
        let firewall_button = if fw.needs_rule() {
            Button::new("firewall-fix")
                .primary()
                .label("Fix")
                .tooltip(format!(
                    "Adds an inbound rule for {}. Windows asks for admin permission once.",
                    st.exe_path
                ))
                .on_click(cx.listener(|this, _, _, _| this.ctl.send(Command::InstallFirewall)))
        } else {
            Button::new("firewall-check")
                .label(match fw {
                    FirewallState::Installing => "Waiting…",
                    FirewallState::Unknown | FirewallState::Checking => "Checking…",
                    _ => "Check again",
                })
                .loading(busy)
                .disabled(busy)
                .tooltip(format!("Rules for {}", st.exe_path))
                .on_click(cx.listener(|this, _, _, _| this.ctl.send(Command::CheckFirewall)))
        };
        vec![
            row("Start with Windows", autostart_desc, start),
            row(
                "Firewall",
                firewall_desc,
                firewall_button.into_any_element(),
            ),
        ]
    }

    fn about_rows(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let open_logs = Button::new("open-logs")
            .label("Open log folder")
            .on_click(cx.listener(|_, _, _, cx| cx.open_with_system(&settings::app_dir())));
        let open_settings = Button::new("open-settings")
            .label("Open settings file")
            .on_click(cx.listener(|_, _, _, cx| {
                let path = settings::settings_path();
                // Saved on the first change; until then there is only the folder.
                cx.open_with_system(if path.exists() {
                    path.as_path()
                } else {
                    path.parent().unwrap_or(&path)
                });
            }));
        vec![
            row(
                "Zephon",
                Desc::plain(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                div().into_any_element(),
            ),
            row(
                "Log",
                Desc::plain(r"%APPDATA%\Zephon\log.txt"),
                open_logs.into_any_element(),
            ),
            row(
                "Settings file",
                Desc::plain(r"%APPDATA%\Zephon\settings.json"),
                open_settings.into_any_element(),
            ),
        ]
    }
}

/// A row's description: one line in Ink 2, or in amber when it is a warning.
struct Desc {
    text: SharedString,
    warn: bool,
}

impl Desc {
    fn plain(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            warn: false,
        }
    }
    fn warn(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            warn: true,
        }
    }
}

/// The label and description of a row with its control; styled by [`group`].
fn row(label: impl Into<SharedString>, desc: Desc, control: AnyElement) -> Div {
    div().child(RowContent {
        label: label.into(),
        desc,
        control: Some(control),
    })
}

#[derive(IntoElement)]
struct RowContent {
    label: SharedString,
    desc: Desc,
    control: Option<AnyElement>,
}

impl RenderOnce for RowContent {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Palette::of(cx);
        let fonts = Fonts::of(cx);
        let narrow = window.viewport_size().width < px(NARROW_WIDTH * fonts.scale);
        let text = v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(2.))
            .child(div().child(self.label))
            .child(
                div()
                    .t_small(&fonts)
                    .text_color(if self.desc.warn { p.amber } else { p.ink_2 })
                    .child(self.desc.text),
            );
        let control = self.control.take().map(|c| {
            div()
                .flex_none()
                .flex()
                .justify_end()
                .when(narrow, |this| this.w_full().justify_start())
                .when(!narrow, |this| this.w(fonts.px(CONTROL_WIDTH)))
                .child(c)
        });
        div()
            .flex()
            .gap(px(16.))
            .when(narrow, |this| this.flex_col().gap(px(10.)))
            .when(!narrow, |this| this.flex_row().items_center())
            .child(text)
            .children(control)
    }
}

/// A section's rows as one rounded group with hairlines, like the rooms list.
fn group(rows: Vec<Div>, p: &Palette) -> impl IntoElement {
    let last = rows.len().saturating_sub(1);
    let rows = rows.into_iter().enumerate().map(|(i, r)| {
        r.w_full()
            .px(px(14.))
            .py(px(12.))
            .bg(p.group_off)
            .when(i > 0, |this| this.border_t_1().border_color(p.stroke))
            .when(i == 0, |this| this.rounded_t(px(7.)))
            .when(i == last, |this| this.rounded_b(px(7.)))
    });
    div()
        .rounded(px(8.))
        .border_1()
        .border_color(p.stroke)
        .child(v_flex().w_full().children(rows))
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::of(cx);
        let fonts = self.fonts.clone();
        let st = self.live.read(cx).state.clone();
        let strip = || div().flex_none().bg(p.root);
        let header = |title: &'static str, first: bool| {
            strip()
                .px(px(16.))
                .pt(px(if first { 4. } else { 22. }))
                .pb(px(6.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title)
        };
        let caption = |text: &'static str| {
            strip()
                .px(px(16.))
                .pb(px(8.))
                .t_small(&fonts)
                .text_color(p.ink_2)
                .child(text)
        };

        let appearance = self.appearance_rows(&p, cx);
        let playback = self.playback_rows(&st, &p, window, cx);
        let rooms = self.room_rows(&st, &p, window, cx);
        let system = self.system_rows(&st, window, cx);
        let about = self.about_rows(cx);
        let rooms_group: AnyElement = if rooms.is_empty() {
            group(
                vec![
                    div()
                        .t_small(&fonts)
                        .text_color(p.ink_2)
                        .child(if st.discovering {
                            "Looking for rooms…"
                        } else {
                            "No rooms found yet."
                        }),
                ],
                &p,
            )
            .into_any_element()
        } else {
            group(rooms, &p).into_any_element()
        };

        v_flex()
            .size_full()
            .t_body(&fonts)
            .text_color(p.ink)
            .child(super::title_bar("Settings", &fonts, &p))
            .child(
                v_flex()
                    .id("settings-content")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(header("Appearance", true))
                    .child(group_on_glass(group(appearance, &p), &p))
                    .child(header("Playback", false))
                    .child(group_on_glass(group(playback, &p), &p))
                    .child(header("Rooms", false))
                    .child(caption(
                        "Hidden rooms are left out of the Rooms list and don't play.",
                    ))
                    .child(group_on_glass(rooms_group, &p))
                    .child(header("System", false))
                    .child(group_on_glass(group(system, &p), &p))
                    .child(header("About", false))
                    .child(group_on_glass(group(about, &p), &p))
                    // The rest of the pane.
                    .child(div().flex_1().min_h(px(20.)).bg(p.root))
                    .overflow_y_scrollbar(),
            )
    }
}
